# justfile

install:
  cargo install --path crates/seshmux-cli --force

demo:
  ./demo/scripts/render.sh
