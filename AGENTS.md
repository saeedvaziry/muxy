# Agents

## Repository

- Read `docs/product/README.md` and `docs/tech/README.md` before changing anything. They define the product and lock the technical design.
- Always ask permission if docs need to be updated
- Before finishing: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`, `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS=-D warnings`.

## Main Guides

- Docs are meant to be high level and simple and clear
- Do not bloat the docs with details
- Code must be self-explanatory

## Third-party dependencies

When adding new third-party dependencies, provide user with their github url and stars and metrics if they are being maintained so user can approve adding it.