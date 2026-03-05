# solana-airdrop

Distribute tokens to thousands of wallets using Merkle proofs. Store one root on-chain, verify claims individually — gas efficient at any scale.

![Rust](https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white)
![Solana](https://img.shields.io/badge/Solana-9945FF?logo=solana&logoColor=white)
![Anchor](https://img.shields.io/badge/Anchor-blue)
![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)

## Features

- Merkle proof verification
- Gas-efficient claims
- Double-claim prevention
- Configurable claim window

## Program Instructions

`initialize` | `claim`

## Build

```bash
anchor build
```

## Test

```bash
anchor test
```

## Deploy

```bash
# Devnet
anchor deploy --provider.cluster devnet

# Mainnet
anchor deploy --provider.cluster mainnet
```

## Project Structure

```
programs/
  solana-airdrop/
    src/
      lib.rs          # Program entry point and instructions
    Cargo.toml
tests/
  solana-airdrop.ts           # Integration tests
Anchor.toml             # Anchor configuration
```

## License

MIT — see [LICENSE](LICENSE) for details.

## Author

Built by [Purple Squirrel Media](https://purplesquirrelmedia.io)
