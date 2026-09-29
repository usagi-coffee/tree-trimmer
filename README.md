# tree-trimmer

Minifies generated Tree-sitter C parsers using macros. Verifies identical preprocessed tokens before saving.

## Install

```sh
cargo install --git https://github.com/usagi-coffee/tree-trimmer
```

## Usage

```sh
tree-trimmer src/parser.c
```

Uses available CPUs, up to 8 workers. Set `--jobs N` to override; `--jobs 1` runs serially.
