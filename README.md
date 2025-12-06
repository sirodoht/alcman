# ālaya

personal book notes

## Setup

```sh
# Compile project (development mode)
cargo build

# Compile and serve on default port :3000
cargo run --bin alcmanserver -- --serve

# Serve with auto-reload
cargo watch -x 'run --bin alcmanserver -- --serve'

# Production mode
cargo run --release --bin alcmanserver -- --serve
```

The server will start at `http://127.0.0.1:3000`

## Scan CLI tool

Optionally configure the OpenAI integration by setting your API key:

```sh
export OPENAI_API_KEY=sk-...
```

Run the scanner to request a one-sentence summary for any book title:

```sh
cargo run --bin alcmanscan "Invisible Cities"
```

### Disable public signups

Set the environment variable below to block new account creation in the web UI:

```sh
export DISABLE_SIGNUPS=1
```

## License

GNU Affero General Public License version 3
