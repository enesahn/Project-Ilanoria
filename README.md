<p align="center">
  <img src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust"/>
  <img src="https://img.shields.io/badge/Solana-9945FF?style=for-the-badge&logo=solana&logoColor=white" alt="Solana"/>
  <img src="https://img.shields.io/badge/Telegram-2CA5E0?style=for-the-badge&logo=telegram&logoColor=white" alt="Telegram"/>
  <img src="https://img.shields.io/badge/Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" alt="Discord"/>
  <img src="https://img.shields.io/badge/Redis-DC382D?style=for-the-badge&logo=redis&logoColor=white" alt="Redis"/>
</p>

<h1 align="center">🚀 Project Ilanoria</h1>

<p align="center">
  <strong>A Telegram/Discord bot that tracks tokens on Solana and executes automatic buys</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-1.0.0-blue?style=flat-square" alt="Version"/>
  <img src="https://img.shields.io/badge/license-Private-red?style=flat-square" alt="License"/>
  <img src="https://img.shields.io/badge/status-Active-success?style=flat-square" alt="Status"/>
</p>

<p align="center">
  <a href="README.tr.md">🇹🇷 Türkçe</a>
</p>

---

## 📖 What Does It Do?

Listens to Telegram or Discord channels, captures token addresses (CA) when someone shares them, and automatically executes buys through the **Bloom API**. Ideal for those who want to follow alpha channels and act fast.

---

## 🏗️ Project Structure

```
src/
├── application/          # Core business logic
│   ├── filter/          # Word filtering (blacklist)
│   ├── health/          # Connection health checks
│   ├── indexer/         # Token indexing engine
│   └── pricing/         # SOL price tracking
├── infrastructure/       # Infrastructure
│   ├── blockchain/      # Blockchain connections (Bloom, GraphQL, RPC)
│   ├── database/        # Redis operations
│   └── logging/         # Log management
└── interfaces/          # User interfaces
    ├── bot/             # Telegram bot (handlers, tasks, ui, user client)
    └── console/         # Console interface
```

---

## ✨ Features

### 🔍 Token Detection & Indexing

| Feature | Description |
|---------|-------------|
| **Shard System** | Token addresses split into 7-character chunks stored in both RAM and Redis |
| **Pumpfun & Raydium** | Real-time tracking of new tokens via WebSocket |
| **LLM Fallback** | Queries Groq API when regex fails to find CA |
| **Blacklist** | Skips messages containing unwanted keywords |

### 📱 Telegram

- ✅ QR code session linking
- ✅ Channel and group monitoring
- ✅ Specific user tracking
- ✅ Automatic invite link joining
- ✅ Markdown formatted notifications

### 💬 Discord

- ✅ WebSocket Gateway connection
- ✅ Channel ID based listening
- ✅ User filtering

### 🌸 Bloom Integration

- ✅ Token purchase operations
- ✅ Wallet management
- ✅ Transaction confirmation tracking via WebSocket
- ✅ Slippage and priority fee settings

### 📋 Task System

Each user can create multiple tasks with:

> Platform, channel, user filter, purchase amount, slippage, priority fee, blacklist, and Bloom wallet selection

### 🖥️ Console Panel

Monitor from terminal:

- 📊 Connection status
- 👤 User logs
- 📝 Task logs (live)
- 🔄 Indexer activity
- 📈 Redis statistics

---

## 🚀 Getting Started

```bash
# Build
cargo build --release

# Run
cargo run --release
```

---

## 📄 License

```
For private use only.
```

---

<p align="center">
  <sub>Built with ❤️ in Rust</sub>
</p>
