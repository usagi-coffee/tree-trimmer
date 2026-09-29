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

Defaults to 16 rounds and up to 8 workers. Override with `--rounds N` and `--jobs N`.
