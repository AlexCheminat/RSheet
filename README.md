# RSheet - Concurrent Spreadsheet Server

A multi-threaded spreadsheet server implementation in Rust, inspired by VisiCalc (1979). RSheet handles concurrent client connections and automatically manages cell dependencies with efficient recalculation.

## Overview

RSheet is a command-driven spreadsheet server that allows multiple clients to simultaneously read and write to a shared spreadsheet. It features automatic dependency tracking and asynchronous cell updates when dependencies change.

## Features

- **Concurrent Access**: Multiple clients can connect and interact with the spreadsheet simultaneously
- **Formula Evaluation**: Uses the Rhai scripting language for cell expressions
- **Dependency Management**: Automatically recalculates dependent cells when source cells change
- **Multi-layered Dependencies**: Supports complex dependency chains (A1 → B1 → C1)
- **Variable Types**: Supports scalar, vector, and matrix cell references
- **Thread-Safe**: Built with Rust's concurrency primitives for safe multi-threaded operation

## Cell Addressing

Cells use A1 notation:
- `A1` - First column, first row
- `B5` - Second column, fifth row
- `AA1` - 27th column, first row
- `ZZ100` - Extended column notation supported

## Commands

### `get <cell>`
Retrieves the value of a cell.

**Examples:**
```
get A1
A1 = None

get B5
B5 = 42
```

### `set <cell> <expression>`
Sets a cell to the result of evaluating an expression.

**Examples:**
```
set A1 42
set B1 2 + 3 * 4
set C1 "hello"
set D1 A1 + B1
```

### `sleep <milliseconds>`
Pauses execution for testing concurrent behavior (primarily for testing purposes).

**Example:**
```
sleep 100
```

## Variable Types

### Scalar Variables
Reference a single cell:
```
set A1 5
set B1 A1 * 2
get B1
B1 = 10
```

### Vector Variables
Reference a row or column of cells using `_` notation:
```
set A1 1
set A2 2
set A3 3
set B1 sum(A1_A3)
get B1
B1 = 6
```

### Matrix Variables
Reference a rectangular region of cells:
```
set A1 1
set A2 2
set B1 3
set B2 4
set C1 sum(A1_B2)
get C1
C1 = 10
```

## Built-in Functions

### `sum(list)`
Calculates the sum of a list, vector, or matrix.

**Examples:**
```
set A1 sum([1, 2, 3, 4])
set B1 sum(A1_A10)
set C1 sum(A1_C3)
```

### `sleep_then(milliseconds, value)`
Waits for specified milliseconds, then returns the value (useful for testing).

**Example:**
```
set A1 sleep_then(1000, 42)
```

## Running the Server

### Interactive Mode
Run without arguments to enter commands directly:
```bash
cargo run
```

### Network Mode
Start a server on a specific address:
```bash
cargo run -- 127.0.0.1:6991
```

Then connect using:
```bash
6991 rsc 127.0.0.1:6991
```

## Multi-Client Testing

Use the sender syntax to simulate multiple clients in interactive mode:
```
client1: set A1 5
client2: get A1
A1 = 5
client1: set B1 A1 * 2
client2: get B1
B1 = 10
```

Each unique sender name creates a new connection.

## Error Handling

The server handles various error conditions:
- **Parse Errors**: Invalid commands or cell references
- **Evaluation Errors**: Invalid expressions (e.g., adding string to number)
- **Dependency Errors**: Cells that depend on cells with errors
- **Invalid Cell References**: Non-existent or malformed cell addresses

**Example:**
```
set A1 "hello"
set B1 A1 + 5
get B1
Error: Cannot add string and number
```

## Dependency Updates

When a cell is updated, all dependent cells are automatically recalculated asynchronously:

```
set A1 5
set B1 A1 * 2
set C1 B1 + 10
get C1
C1 = 20

set A1 10
sleep 50
get C1
C1 = 30
```

The `sleep` ensures enough time for background recalculation to complete.

## Technical Details

- **Concurrency Model**: Each client connection runs in its own thread
- **Background Worker**: A dedicated thread handles dependency recalculation
- **Atomic Operations**: Set commands complete atomically
- **Race Condition Handling**: Ensures newer updates aren't overwritten by slower, older ones

## Cell Value Types

Cells can contain:
- **None**: Default empty value
- **Integer** (`i64`): Numeric values
- **String**: Text values
- **Error**: Error messages from failed evaluations
