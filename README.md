<p align="center">
  <img src="docs/banner.svg" alt="OpenWiki Banner" width="100%"/>
</p>

<p align="center">
  <a href="https://github.com/kdsz001/OpenWiki/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-F97316?style=flat-square" alt="License"></a>
  <a href="https://github.com/kdsz001/OpenWiki/releases"><img src="https://img.shields.io/github/v/release/kdsz001/OpenWiki?style=flat-square&color=F97316" alt="Release"></a>
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-F97316?style=flat-square" alt="Platform">
  <img src="https://img.shields.io/badge/PRs-welcome-F97316?style=flat-square" alt="PRs Welcome">
</p>

<p align="center">
  Copy anything → popup appears on desktop → choose to keep → AI organizes it into a knowledge base<br>
  <b>You decide what to keep. AI makes sense of it.</b>
</p>

<p align="center">
  Privacy first — all data stored in local SQLite database.
</p>

> **Network transparency:** URL reading can send the full URL to Jina Reader, and the Chinese UI can send foreign-language page text to Google Translate. Both are enabled by default for compatibility and can be disabled independently in Settings. Platform-specific readers may still contact the source platform or its API.

<p align="center">
  <a href="https://openwiki.pages.dev">🌐 Website</a> · <a href="README.zh-CN.md">中文文档</a>
</p>

## Screenshots

| Content Capture | Knowledge Base |
|:---:|:---:|
| ![Content](docs/screenshots/content.png) | ![Wiki](docs/screenshots/wiki.png) |

| Knowledge Graph | Deep Insights |
|:---:|:---:|
| ![Graph](docs/screenshots/graph.png) | ![Insights](docs/screenshots/insights.png) |

## Features

### 📋 Capture Popup
- A popup appears on your desktop when you copy something (auto-dismisses after 10 seconds)
- **Only content you actively choose to keep gets saved** — no silent hoarding
- Supports text, images, and URLs with automatic source app detection
- Fetches full article content from WeChat, X/Twitter, and other URLs
- `⌘⇧C` on macOS or `Ctrl+Shift+C` on Windows to manually trigger the capture window

### 📂 Content Management
- Filter by type (text / image / link) and time range
- Global search across content and knowledge base
- One-click export to Markdown

### 🧠 AI Knowledge Base
- AI automatically compiles captured content into Wiki pages (concepts, entities, topics)
- Knowledge graph visualization — see how ideas connect
- **Ask sidebar** — ask questions about your knowledge base, AI answers based on your content
- Auto-detect orphaned pages, broken links, and structural issues

### 📊 Insight Reports
- One-click AI weekly report summarizing captured content
- **Attention analysis** — 7-dimension insights into your information habits:
    - At a Glance / Subconscious / Graveyard / Blind Spots / Hot Topics / Heatmap / Action Items
- Like or dismiss report items — AI learns your preferences

### ⚙️ AI Providers
- Supports **Anthropic (Claude)** / **OpenAI** / **Google Gemini**
- API Key or OAuth login — two ways to connect
- Choose different models for each provider

### 🖥 Desktop Experience
- System tray — closing the window keeps the app running
- `⌘⇧Y` on macOS or `Ctrl+Shift+Y` on Windows to show the main window
- Dark / Light / System theme
- MCP protocol integration — connect to Claude Desktop

## Download

- macOS (Apple Silicon): download `OpenWiki_X.Y.Z_aarch64.dmg`
- macOS (Intel): download `OpenWiki_X.Y.Z_x64.dmg`
- Windows (x64): download `OpenWiki_X.Y.Z_x64-setup.exe` (recommended) or `OpenWiki_X.Y.Z_x64_en-US.msi`

👉 [Go to Releases](https://github.com/kdsz001/OpenWiki/releases)

### ⚠️ First Launch Guide (Important)

Choose the steps for your operating system.

#### macOS

The app is signed and notarized by Apple — just double-click to open, no security bypass needed:

1. Open the `.dmg` and drag OpenWiki into the Applications folder
2. Launch the app and click "Allow" in the authorization prompt
3. Go to Settings → AI to configure your AI provider

#### Windows

The Windows build is unsigned, so Microsoft Defender SmartScreen may warn on first launch:

1. Run `OpenWiki_X.Y.Z_x64-setup.exe`
2. If SmartScreen appears, choose **More info** → **Run anyway**
3. Launch OpenWiki from the Start menu or desktop shortcut
4. Go to Settings → AI to configure your AI provider

## Development

### Prerequisites
- Node.js 18+
- Rust (latest stable)
- macOS 13+ or Windows 10/11
- macOS: Xcode Command Line Tools (`xcode-select --install`)
- Windows: Microsoft C++ Build Tools / Visual Studio Build Tools and WebView2 Runtime

### Getting Started

```bash
# Clone the repo
git clone https://github.com/kdsz001/OpenWiki.git
cd OpenWiki

# Install dependencies
npm install

# Development mode
npm run tauri dev

# Build the app
npm run tauri build
```

Before creating release bundles, prepare the bundled document converter:

```bash
# macOS / Linux
./src-tauri/scripts/setup_markitdown.sh

# Windows PowerShell
./src-tauri/scripts/setup_markitdown.ps1
```

## Contributing

Contributions welcome! Read [CONTRIBUTING.md](CONTRIBUTING.md) for development workflow and guidelines.

## Acknowledgments

- [Andrej Karpathy](https://github.com/karpathy) — his [LLM Wiki idea](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) inspired the knowledge base design

## Special Thanks

Thanks to everyone who helped spread the word:

- [@NFTCPS](https://x.com/NFTCPS)

## Author

**Ray** — [@BitcoinRui](https://x.com/BitcoinRui)

## License

[MIT](LICENSE)

## Star History

[![Star History Chart](https://star-history.dera.page/svg?repos=kdsz001/OpenWiki&type=date&legend=top-left)](https://star-history.dera.page/#kdsz001/OpenWiki&type=date&legend=top-left)
