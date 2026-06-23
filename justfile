set dotenv-load

# flash the demo example
demo:
    cargo run --release --example demo

# run the music server
server:
    cd server && cargo run

# fetch and display now-playing over wifi
fetch:
    SSID="$SSID" PASSWORD="$PASSWORD" SERVER_URL="$SERVER_URL" cargo run --release --example fetch --features wifi

# check firmware compiles
check:
    cargo c --example demo

# format and lint everything
lint:
    cargo fmt
    cargo clippy --all-features
    cd server && cargo fmt
    cd server && cargo clippy
