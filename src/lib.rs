// Alexandre Cheminat - z5592322 - 14/11/25

use rsheet_lib::cell_expr::{CellArgument, CellExpr};
use rsheet_lib::cell_value::CellValue;
use rsheet_lib::cells::column_number_to_name;
use rsheet_lib::command::{CellIdentifier, Command};
use rsheet_lib::connect::{
    Connection, Manager, ReadMessageResult, Reader, WriteMessageResult, Writer,
};
use rsheet_lib::replies::Reply;

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone, Debug)]
enum CellState {
    Value(CellValue),
    DependencyError,
}

#[derive(Clone, Debug)]
struct CellData {
    expr: String,
    state: CellState,
    version: u64,
}

struct Spreadsheet {
    cells: HashMap<String, CellData>,
    dependents: HashMap<String, HashSet<String>>,
    version_counter: Arc<AtomicU64>,
}

enum SetPrep {
    Error(()),
    DependencyError,
    ReadyToEvaluate(
        Box<CellExpr>,
        HashMap<String, CellArgument>,
        HashSet<String>,
    ),
}

impl Spreadsheet {
    fn new(version_counter: Arc<AtomicU64>) -> Self {
        Spreadsheet {
            cells: HashMap::new(),
            dependents: HashMap::new(),
            version_counter,
        }
    }

    fn get(&self, cell_identifier: &CellIdentifier) -> Reply {
        let key = cell_identifier_to_string(cell_identifier);
        match self.cells.get(&key) {
            Some(cell_data) => match &cell_data.state {
                CellState::Value(value) => Reply::Value(key.clone(), value.clone()),
                CellState::DependencyError => {
                    Reply::Error(format!("Cell {} depends on a cell with an error", key))
                }
            },
            None => Reply::Value(key, CellValue::None),
        }
    }

    fn prepare_set(
        &self,
        cell_identifier: &CellIdentifier,
        cell_expr: String,
    ) -> (String, SetPrep, u64) {
        let key = cell_identifier_to_string(cell_identifier);
        let version = self.version_counter.fetch_add(1, Ordering::SeqCst);
        let expr = CellExpr::new(&cell_expr);
        let dependencies = expr.find_variable_names();

        let mut dep_args = HashMap::new();
        let mut has_error_dependency = false;
        let mut direct_deps = HashSet::new();

        for dep in dependencies {
            let arg = if dep.contains('_') {
                match self.parse_range(&dep, &mut direct_deps, &mut has_error_dependency) {
                    Ok(arg) => arg,
                    Err(_) => return (key, SetPrep::Error(()), version),
                }
            } else {
                direct_deps.insert(dep.clone());
                CellArgument::Value(self.get_cell_value(&dep, &mut has_error_dependency))
            };

            dep_args.insert(dep, arg);
        }

        if has_error_dependency {
            (key, SetPrep::DependencyError, version)
        } else {
            (
                key,
                SetPrep::ReadyToEvaluate(Box::new(expr), dep_args, direct_deps),
                version,
            )
        }
    }

    fn parse_range(
        &self,
        range: &str,
        direct_deps: &mut HashSet<String>,
        has_error_dependency: &mut bool,
    ) -> Result<CellArgument, String> {
        let parts: Vec<&str> = range.split('_').collect();
        if parts.len() != 2 {
            return Err(format!("Invalid range format: {}", range));
        }

        let start = parts[0]
            .parse::<CellIdentifier>()
            .map_err(|_| format!("Invalid cell identifier: {}", parts[0]))?;
        let end = parts[1]
            .parse::<CellIdentifier>()
            .map_err(|_| format!("Invalid cell identifier: {}", parts[1]))?;

        let is_single_row = start.row == end.row;
        let is_single_col = start.col == end.col;

        if is_single_row {
            Ok(CellArgument::Vector(self.collect_row(
                start.row,
                start.col,
                end.col,
                direct_deps,
                has_error_dependency,
            )))
        } else if is_single_col {
            Ok(CellArgument::Vector(self.collect_col(
                start.col,
                start.row,
                end.row,
                direct_deps,
                has_error_dependency,
            )))
        } else {
            Ok(CellArgument::Matrix(self.collect_matrix(
                start,
                end,
                direct_deps,
                has_error_dependency,
            )))
        }
    }

    fn collect_row(
        &self,
        row: u32,
        start_col: u32,
        end_col: u32,
        direct_deps: &mut HashSet<String>,
        has_error: &mut bool,
    ) -> Vec<CellValue> {
        (start_col..=end_col)
            .map(|col| {
                let key = cell_identifier_to_string(&CellIdentifier { col, row });
                direct_deps.insert(key.clone());
                self.get_cell_value(&key, has_error)
            })
            .collect()
    }

    fn collect_col(
        &self,
        col: u32,
        start_row: u32,
        end_row: u32,
        direct_deps: &mut HashSet<String>,
        has_error: &mut bool,
    ) -> Vec<CellValue> {
        (start_row..=end_row)
            .map(|row| {
                let key = cell_identifier_to_string(&CellIdentifier { col, row });
                direct_deps.insert(key.clone());
                self.get_cell_value(&key, has_error)
            })
            .collect()
    }

    fn collect_matrix(
        &self,
        start: CellIdentifier,
        end: CellIdentifier,
        direct_deps: &mut HashSet<String>,
        has_error: &mut bool,
    ) -> Vec<Vec<CellValue>> {
        (start.row..=end.row)
            .map(|row| {
                (start.col..=end.col)
                    .map(|col| {
                        let key = cell_identifier_to_string(&CellIdentifier { col, row });
                        direct_deps.insert(key.clone());
                        self.get_cell_value(&key, has_error)
                    })
                    .collect()
            })
            .collect()
    }

    fn get_cell_value(&self, cell_key: &str, has_error_dependency: &mut bool) -> CellValue {
        match self.cells.get(cell_key) {
            Some(cell_data) => match &cell_data.state {
                CellState::Value(v) => {
                    if matches!(v, CellValue::Error(_)) {
                        *has_error_dependency = true;
                    }
                    v.clone()
                }
                CellState::DependencyError => {
                    *has_error_dependency = true;
                    CellValue::None
                }
            },
            None => CellValue::None,
        }
    }

    fn complete_set(
        &mut self,
        key: String,
        expr: String,
        result: Result<CellValue, ()>,
        direct_deps: HashSet<String>,
        version: u64,
    ) -> bool {
        // Check if this version is still valid
        if let Some(existing) = self.cells.get(&key) {
            if existing.version > version {
                return false;
            }
        }

        // Remove old dependencies
        for (_, deps) in self.dependents.iter_mut() {
            deps.remove(&key);
        }

        for dep in direct_deps {
            self.dependents.entry(dep).or_default().insert(key.clone());
        }

        let state = match result {
            Ok(value) => CellState::Value(value),
            Err(_) => CellState::DependencyError,
        };

        self.cells.insert(
            key,
            CellData {
                expr,
                state,
                version,
            },
        );
        true
    }

    // Get all cells that need to be updated
    fn get_cells_to_update(&self, changed_cell: &str) -> Vec<String> {
        let mut to_update = HashSet::new();
        let mut queue = vec![changed_cell.to_string()];

        while let Some(cell) = queue.pop() {
            if let Some(deps) = self.dependents.get(&cell) {
                for dependent in deps {
                    if to_update.insert(dependent.clone()) {
                        queue.push(dependent.clone());
                    }
                }
            }
        }

        self.topological_sort(to_update)
    }

    // Sort to ensure dependencies are evaluated before dependents
    fn topological_sort(&self, cells: HashSet<String>) -> Vec<String> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut temp_mark = HashSet::new();

        for cell in &cells {
            if !visited.contains(cell) {
                self.topological_visit(cell, &cells, &mut visited, &mut temp_mark, &mut result);
            }
        }

        result
    }

    fn topological_visit(
        &self,
        cell: &str,
        cells_to_sort: &HashSet<String>,
        visited: &mut HashSet<String>,
        temp_mark: &mut HashSet<String>,
        result: &mut Vec<String>,
    ) {
        if visited.contains(cell) {
            return;
        }

        if temp_mark.contains(cell) {
            return;
        }

        temp_mark.insert(cell.to_string());

        // Visit all dependencies of this cell that are in our set
        if let Some(cell_data) = self.cells.get(cell) {
            let expr = CellExpr::new(&cell_data.expr);
            let dependencies = expr.find_variable_names();

            for dep in dependencies {
                if cells_to_sort.contains(&dep) && !visited.contains(&dep) {
                    self.topological_visit(&dep, cells_to_sort, visited, temp_mark, result);
                }
            }
        }

        temp_mark.remove(cell);
        visited.insert(cell.to_string());
        result.push(cell.to_string());
    }
}

pub fn start_server<M>(mut manager: M) -> Result<(), Box<dyn Error>>
where
    M: Manager,
{
    let version_counter = Arc::new(AtomicU64::new(0));
    let spreadsheet = Arc::new(Mutex::new(Spreadsheet::new(Arc::clone(&version_counter))));
    let mut handles = vec![];

    while let Connection::NewConnection { reader, writer } = manager.accept_new_connection() {
        let spreadsheet_clone = Arc::clone(&spreadsheet);

        let handle = thread::spawn(move || handle_connection(reader, writer, spreadsheet_clone));

        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.join();
    }

    Ok(())
}

fn handle_connection<R, W>(mut recv: R, mut send: W, spreadsheet: Arc<Mutex<Spreadsheet>>)
where
    R: Reader,
    W: Writer,
{
    loop {
        match recv.read_message() {
            ReadMessageResult::Message(msg) => {
                if let Some(reply) = handle_command(&msg, &spreadsheet) {
                    if !send_reply(&mut send, reply) {
                        break;
                    }
                }
            }
            ReadMessageResult::ConnectionClosed => break,
            ReadMessageResult::Err(e) => {
                eprintln!("Error reading message: {:?}", e);
                break;
            }
        }
    }
}

fn handle_command(msg: &str, spreadsheet: &Arc<Mutex<Spreadsheet>>) -> Option<Reply> {
    match msg.parse::<Command>() {
        Ok(Command::Get { cell_identifier }) => {
            let sheet = spreadsheet.lock().unwrap();
            Some(sheet.get(&cell_identifier))
        }
        Ok(Command::Set {
            cell_identifier,
            cell_expr,
        }) => {
            handle_set_command(spreadsheet, &cell_identifier, cell_expr);
            None
        }
        Err(e) => Some(Reply::Error(e)),
    }
}

fn handle_set_command(
    spreadsheet: &Arc<Mutex<Spreadsheet>>,
    cell_identifier: &CellIdentifier,
    cell_expr: String,
) {
    // Gather dependencies while holding lock
    let (key, prep, version) = {
        let sheet = spreadsheet.lock().unwrap();
        sheet.prepare_set(cell_identifier, cell_expr.clone())
    };

    match prep {
        SetPrep::DependencyError => {
            let mut sheet = spreadsheet.lock().unwrap();
            sheet.complete_set(key, cell_expr, Err(()), HashSet::new(), version);
        }
        SetPrep::ReadyToEvaluate(expr, dep_args, direct_deps) => {
            // Evaluate without holding lock
            let result = expr.evaluate(&dep_args);

            // Store result and get dependent cells
            let cells_to_update = {
                let mut sheet = spreadsheet.lock().unwrap();
                let updated = sheet.complete_set(
                    key.clone(),
                    cell_expr,
                    result.map_err(|_| ()),
                    direct_deps,
                    version,
                );

                if updated {
                    sheet.get_cells_to_update(&key)
                } else {
                    Vec::new()
                }
            };

            // Re-evaluate all dependent cells
            for cell_to_update in cells_to_update {
                update_dependent_cell(spreadsheet, cell_to_update);
            }
        }
        SetPrep::Error(_) => (),
    }
}

fn update_dependent_cell(spreadsheet: &Arc<Mutex<Spreadsheet>>, cell_key: String) {
    // Get the cell's expression and prepare for update
    let (expr_str, prep, version) = {
        let sheet = spreadsheet.lock().unwrap();
        let Some(cell_data) = sheet.cells.get(&cell_key) else {
            return;
        };

        let expr_str = cell_data.expr.clone();
        let Ok(cell_id) = cell_key.parse::<CellIdentifier>() else {
            return;
        };

        let (_, prep, version) = sheet.prepare_set(&cell_id, expr_str.clone());
        (expr_str, prep, version)
    };

    // Handle update
    match prep {
        SetPrep::Error(_) => {}
        SetPrep::DependencyError => {
            let mut sheet = spreadsheet.lock().unwrap();
            sheet.complete_set(cell_key, expr_str, Err(()), HashSet::new(), version);
        }
        SetPrep::ReadyToEvaluate(expr, dep_args, direct_deps) => {
            let result = expr.evaluate(&dep_args);
            let mut sheet = spreadsheet.lock().unwrap();
            sheet.complete_set(
                cell_key,
                expr_str,
                result.map_err(|_| ()),
                direct_deps,
                version,
            );
        }
    }
}

fn send_reply<W: Writer>(send: &mut W, reply: Reply) -> bool {
    match send.write_message(reply) {
        WriteMessageResult::Ok => true,
        WriteMessageResult::ConnectionClosed => false,
        WriteMessageResult::Err(e) => {
            eprintln!("Error writing message: {:?}", e);
            false
        }
    }
}

fn cell_identifier_to_string(cell_id: &CellIdentifier) -> String {
    format!("{}{}", column_number_to_name(cell_id.col), cell_id.row + 1)
}
