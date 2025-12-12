# Project Ilanoria

Solana üzerinde token takibi yapıp otomatik alım atan bir Telegram/Discord botu. Rust ile yazıldı, hızlı çalışıyor.

## Ne İş Yapıyor?

Telegram veya Discord kanallarını dinliyor, birisi token adresi (CA) paylaştığında bunu yakalayıp Bloom API üzerinden otomatik alım yapıyor. Alfa kanallarını takip edip hızlı hareket etmek isteyenler için ideal.

## Proje Yapısı

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

## Özellikler

### Token Tespit ve İndeksleme

- **Shard sistemi**: Token adresleri 7 karakterlik parçalara bölünüp hem RAM'de hem Redis'te tutuluyor. Mesaj içinde CA aramak bu sayede çok hızlı oluyor.
- **Pumpfun ve Raydium desteği**: WebSocket üzerinden yeni tokenları anlık takip ediyor.
- **LLM yedek**: Normal regex ile CA bulunamazsa Groq API'ye soruyor.
- **Blacklist**: İstenmeyen kelimeleri içeren mesajları atlıyor.

### Telegram

- QR kod ile oturum bağlama
- Kanal ve grup dinleme
- Belirli kullanıcıları izleme
- Otomatik davet linklerine katılma
- Markdown formatında bildirimler

### Discord

- WebSocket ile Gateway bağlantısı
- Kanal ID'ye göre dinleme
- Kullanıcı filtreleme

### Bloom Entegrasyonu

- Token alım işlemleri
- Cüzdan yönetimi
- WebSocket ile işlem onayı takibi
- Slippage ve priority fee ayarları

### Görev Sistemi

Her kullanıcı birden fazla görev oluşturabiliyor. Her görevde platform, kanal, kullanıcı filtresi, alım miktarı, slippage, priority fee, blacklist ve Bloom cüzdan seçimi yapılabiliyor.

### Konsol Paneli

Terminal üzerinden bağlantı durumları, kullanıcı logları, görev logları (canlı), indeksleyici aktivitesi ve Redis istatistikleri görüntülenebiliyor.

## Çalıştırma

```bash
cargo build --release
cargo run --release
```

## Lisans

Özel kullanım için.
