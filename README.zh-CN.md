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
  复制任何内容 → 桌面弹出浮窗 → 选择收藏 → AI 自动整理成知识库<br>
  <b>你决定留什么，AI 帮你理清楚。</b>
</p>

<p align="center">
  隐私优先 — 所有数据存储在本地 SQLite 数据库。
</p>

> **联网说明：** 链接读取可能把完整 URL 发送给 Jina Reader；中文界面的外文翻译可能把网页正文发送给 Google Translate。两项默认开启以保持原有体验，可在设置中分别关闭。平台专用读取器仍可能访问对应平台或 API。

<p align="center">
  <a href="https://openwiki.pages.dev">🌐 官网</a>
</p>

## 📸 截图

| 内容捕获 | 知识库 |
|:---:|:---:|
| ![内容](docs/screenshots/content.png) | ![知识库](docs/screenshots/wiki.png) |

| 知识图谱 | 深度洞察 |
|:---:|:---:|
| ![图谱](docs/screenshots/graph.png) | ![洞察](docs/screenshots/insights.png) |

## 核心功能

### 📋 捕获浮窗
- 复制内容时桌面弹出浮窗（默认 10 秒后消失）
- **只有你主动选择收藏的内容才会保存**，不会偷偷囤积
- 支持文本、图片、URL，自动识别来源应用
- 支持抓取微信公众号、X/Twitter 等 URL 的正文内容
- macOS 使用 `⌘⇧C`、Windows 使用 `Ctrl+Shift+C` 可随时手动呼出捕获窗口

### 📂 内容管理
- 按类型（文本 / 图片 / 链接）和时间范围过滤
- 全局搜索，跨内容和知识库同时检索
- 一键导出为 Markdown 文件

### 🧠 AI 知识库
- AI 自动将捕获内容编译为 Wiki 页面（概念、实体、主题）
- 知识图谱可视化，看见概念之间的关联
- **Ask 侧栏** — 向你的知识库提问，AI 基于你的内容回答
- 自动检测孤立页面、断裂链接等结构问题

### 📊 洞察报告
- 一键生成 AI 周报，汇总本周捕获内容
- **注意力分析** — 7 维度洞察你的信息习惯：
    - 一瞥总览 / 潜意识 / 遗忘墓地 / 盲区 / 热点 / 热力图 / 行动建议
- 对报告内容点赞或忽略，AI 学习你的偏好

### ⚙️ AI 提供商
- 支持 **Anthropic (Claude)** / **OpenAI** / **Google Gemini**
- API Key 或 OAuth 登录，两种接入方式
- 可为每个提供商选择不同模型

### 🖥 桌面体验
- 系统托盘常驻，关闭窗口不退出
- macOS 使用 `⌘⇧Y`、Windows 使用 `Ctrl+Shift+Y` 唤起主窗口
- 深色 / 浅色 / 跟随系统主题
- MCP 协议集成，可连接 Claude Desktop

## 下载安装

- macOS (Apple Silicon): 下载下方的 `OpenWiki_X.Y.Z_aarch64.dmg`
- macOS (Intel): 下载下方的 `OpenWiki_X.Y.Z_x64.dmg`
- Windows (x64): 下载 `OpenWiki_X.Y.Z_x64-setup.exe`（推荐）或 `OpenWiki_X.Y.Z_x64_en-US.msi`

👉 [前往 Release 页面下载](https://github.com/kdsz001/OpenWiki/releases)

### ⚠️ 首次打开指南（重要）

请根据你的系统选择对应步骤。

#### macOS

应用已完成 Apple 签名与公证，双击即可打开，不会被系统拦截：

1. 打开 `.dmg`，将 OpenWiki 拖入「应用程序」文件夹
2. 运行应用，在弹出的授权窗口点击“允许”
3. 在应用的“设置”->“AI”中配置 AI 提供商信息

#### Windows

Windows 版本暂未进行代码签名，首次运行时 Microsoft Defender SmartScreen 可能会提示风险：

1. 运行 `OpenWiki_X.Y.Z_x64-setup.exe`
2. 如果出现 SmartScreen 提示，点击“更多信息”->“仍要运行”
3. 从开始菜单或桌面快捷方式启动 OpenWiki
4. 在应用的“设置”->“AI”中配置 AI 提供商信息

## 开发指南

### 前置要求
- Node.js 18+
- Rust (最新 stable)
- macOS 13+ 或 Windows 10/11
- macOS: Xcode Command Line Tools （终端运行 `xcode-select --install`）
- Windows: Microsoft C++ Build Tools / Visual Studio Build Tools 和 WebView2 Runtime

### 开始

```bash
# 克隆仓库
git clone https://github.com/kdsz001/OpenWiki.git
cd OpenWiki

# 安装依赖
npm install

# 开发模式
npm run tauri dev

# 构建应用
npm run tauri build
```

创建正式安装包前，需要先准备内置文档转换器：

```bash
# macOS / Linux
./src-tauri/scripts/setup_markitdown.sh

# Windows PowerShell
./src-tauri/scripts/setup_markitdown.ps1
```

## 参与贡献

欢迎贡献！请阅读 [CONTRIBUTING.md](CONTRIBUTING.md) 了解开发流程和规范。

## 致谢

- [Andrej Karpathy](https://github.com/karpathy) — 他的 [LLM Wiki 构想](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) 启发了知识库的设计

## 特别鸣谢

感谢每一位帮忙传播 OpenWiki 的朋友：

- [@NFTCPS](https://x.com/NFTCPS)

## 作者

**Ray** — [@BitcoinRui](https://x.com/BitcoinRui)

## License

[MIT](LICENSE)

## Star History

[![Star History Chart](https://star-history.dera.page/svg?repos=kdsz001/OpenWiki&type=date&legend=top-left)](https://star-history.dera.page/#kdsz001/OpenWiki&type=date&legend=top-left)
