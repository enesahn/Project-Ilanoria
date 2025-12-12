<p align="center">
  <img src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust"/>
  <img src="https://img.shields.io/badge/Solana-9945FF?style=for-the-badge&logo=solana&logoColor=white" alt="Solana"/>
  <img src="https://img.shields.io/badge/Telegram-2CA5E0?style=for-the-badge&logo=telegram&logoColor=white" alt="Telegram"/>
  <img src="https://img.shields.io/badge/Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" alt="Discord"/>
  <img src="https://img.shields.io/badge/Redis-DC382D?style=for-the-badge&logo=redis&logoColor=white" alt="Redis"/>
</p>

<h1 align="center">🚀 Project Ilanoria</h1>

<p align="center">
  <strong>Solana üzerinde token takibi yapıp otomatik alım atan bir Telegram/Discord botu</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-1.0.0-blue?style=flat-square" alt="Version"/>
  <img src="https://img.shields.io/badge/license-Private-red?style=flat-square" alt="License"/>
  <img src="https://img.shields.io/badge/status-Active-success?style=flat-square" alt="Status"/>
</p>

<p align="center">
  <a href="README.md">🇬🇧 English</a>
</p>

---

## 📖 Ne İş Yapıyor?

Telegram veya Discord kanallarını dinliyor, birisi token adresi (CA) paylaştığında bunu yakalayıp **Bloom API** üzerinden otomatik alım yapıyor. Alfa kanallarını takip edip hızlı hareket etmek isteyenler için ideal.

---

## 🏗️ Proje Yapısı

```
src/
├── application/          # Ana iş mantığı
│   ├── filter/          # Kelime filtreleme (blacklist)
│   ├── health/          # Bağlantı kontrolü
│   ├── indexer/         # Token indeksleme motoru
│   └── pricing/         # SOL fiyat takibi
├── infrastructure/       # Altyapı
│   ├── blockchain/      # Blockchain bağlantıları (Bloom, GraphQL, RPC)
│   ├── database/        # Redis işlemleri
│   └── logging/         # Log yönetimi
└── interfaces/          # Kullanıcı arayüzleri
    ├── bot/             # Telegram bot (handlers, tasks, ui, user client)
    └── console/         # Konsol arayüzü
```

---

## ✨ Özellikler

### 🔍 Token Tespit ve İndeksleme

| Özellik | Açıklama |
|---------|----------|
| **Shard Sistemi** | Token adresleri 7 karakterlik parçalara bölünüp hem RAM'de hem Redis'te tutuluyor |
| **Pumpfun & Raydium** | WebSocket üzerinden yeni tokenları anlık takip |
| **LLM Yedek** | Normal regex ile CA bulunamazsa Groq API'ye soruyor |
| **Blacklist** | İstenmeyen kelimeleri içeren mesajları atlıyor |

### 📱 Telegram

- ✅ QR kod ile oturum bağlama
- ✅ Kanal ve grup dinleme
- ✅ Belirli kullanıcıları izleme
- ✅ Otomatik davet linklerine katılma
- ✅ Markdown formatında bildirimler

### 💬 Discord

- ✅ WebSocket ile Gateway bağlantısı
- ✅ Kanal ID'ye göre dinleme
- ✅ Kullanıcı filtreleme

### 🌸 Bloom Entegrasyonu

- ✅ Token alım işlemleri
- ✅ Cüzdan yönetimi
- ✅ WebSocket ile işlem onayı takibi
- ✅ Slippage ve priority fee ayarları

### 📋 Görev Sistemi

Her kullanıcı birden fazla görev oluşturabiliyor:

> Platform, kanal, kullanıcı filtresi, alım miktarı, slippage, priority fee, blacklist ve Bloom cüzdan seçimi

### 🖥️ Konsol Paneli

Terminal üzerinden izlenebilir:

- 📊 Bağlantı durumları
- 👤 Kullanıcı logları
- 📝 Görev logları (canlı)
- 🔄 İndeksleyici aktivitesi
- 📈 Redis istatistikleri

---

## 🚀 Çalıştırma

```bash
# Build
cargo build --release

# Run
cargo run --release
```

---

## 📄 Lisans

```
Özel kullanım için.
```

---

<p align="center">
  <sub>Built with ❤️ in Rust</sub>
</p>
