set dotenv-load

# flash the demo example
demo:
    cargo run --release --example demo

# run the music server
server:
    cd server && cargo run

# check firmware compiles
check:
    cargo c --example demo

# format and lint everything
lint:
    cargo fmt
    cargo clippy --all-features
    cd server && cargo fmt
    cd server && cargo clippy
