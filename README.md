# KNU Diagnosis — Auto Answer Filler

Native Windows System Tray assistant written in **Rust + Win32 API** that silently captures selected on-screen questions, sends them directly to **OpenRouter Vision AI** (without slow or error-prone local OCR), optionally verifies Python answers in an isolated subprocess, and automatically fills matching answer fields via **Windows UI Automation**—without ever clicking submit.

## Workflow

`Global Shortcut (Alt+Z) → Transparent Drag-to-Select Overlay (Esc to Cancel) → Cropped Screen Capture → OpenRouter Vision AI (Single or Multiple Questions) → Optional Isolated Python Verification → Scoped UIA Detect & Safe Auto-Fill → Last Q&A Viewer (Alt+X) / Copy Answers (Alt+C) / Copy Explanations (Ctrl+Alt+C)`

## Features

- **Transparent Region Selection (`Alt+Z`)**: Press `Alt+Z` to start a silent, transparent full-screen selection drag over one or multiple questions (`Esc` or right-click cancels).
- **Multi-Question & Multi-Gap Auto-Fill**: Detects MCQ radio options (`o`/`0`), fill-in-the-blank boxes (above, below, or inline `(1)`/`1`), checkboxes, dropdowns, and code editors via Vision target coordinates + Windows UI Automation safety guards. Never clicks Submit/Finish buttons.
- **Fast-Scan Last Q&A Viewer (`Alt+X`)**: Press `Alt+X` or left-click the system tray icon to toggle the Last Q&A dialog:
  - Correct answers are rendered **bold and 2pt larger**.
  - Explanation text and Python executed `stdout` (`PythonExecuted (stdout: ...)`) are rendered **bold** for instant visual scanning if manual entry is needed.
- **Global Copy Shortcuts**:
  - `Alt+C`: Copy the solved answer(s) directly to the Windows clipboard (even with the Q&A dialog closed).
  - `Ctrl+Alt+C`: Copy all explanations from the last Q&A result to the Windows clipboard.
- **Press-to-Detect Hotkey Recorder**: Click any shortcut box in Settings and press your preferred key combination to rebind any hotkey.
- **Frontier OpenRouter Model Presets + Custom Entry**:
  - **Vision Model presets**: `z-ai/glm-5.3-flash` *(default)*, `anthropic/claude-fable-5.2`, `anthropic/claude-opus-5.5`, `openai/gpt-6.1-terra`, `openai/gpt-6.1-sol`, `openai/gpt-6-astra`, `google/gemini-4-carbon`, `google/gemini-2.5-flash`, `qwen/qwen4-max`, `x-ai/grok-4.7`, plus `Custom...`.
  - **Reasoning Model presets**: `openai/gpt-4o-mini` *(default)*, `anthropic/claude-fable-5.2`, `anthropic/claude-opus-5.5`, `openai/gpt-6-astra`, `openai/gpt-6.1-sol`, `google/gemini-4-argon`, `google/gemini-4-carbon`, `deepseek/deepseek-v4-pro-0813`, `deepseek/deepseek-v4.1-flash`, `qwen/qwen4-max`, `z-ai/glm-5.3-prime`, `x-ai/grok-4.7`, plus `Custom...`.
- **Secure API Key Storage**: Stores your OpenRouter API key in **Windows Credential Manager** (`KNUDiagnosis/OpenRouterApiKey`), never in plain text or SQLite logs.
- **Searchable SQLite History**: Saves solved Q&A history locally under `%LOCALAPPDATA%\KNUDiagnosis\history.db`.
- **Isolated Python Verification**: Verifies Python code snippets in an isolated `python -I -B` subprocess with stripped environment variables and a hard timeout.

## Installation (Windows)

1. Download **`KNUDiagnosis-Setup-0.1.0.exe`** from the [GitHub Releases](../../releases) page.
2. Run the installer (user-level install in `%LOCALAPPDATA%\Programs\KNUDiagnosis`, no administrator privileges required).
3. Right-click the **KNU Diagnosis** icon in the Windows notification tray → **Settings...**.
4. Enter your **OpenRouter API Key**, adjust your Vision/Reasoning models and shortcuts if desired, and click **Save**.
5. Press `Alt+Z` and drag across one or more questions on screen.

## Default Shortcuts

| Action | Default Shortcut |
| --- | --- |
| Transparent Capture & Auto-Fill | `Alt+Z` |
| Show / Hide Last Q&A Viewer | `Alt+X` |
| Copy Last Answer(s) | `Alt+C` |
| Copy Last Explanation(s) | `Ctrl+Alt+C` |

## Building from Source (Windows)

### Prerequisites

- Rust (stable, `x86_64-pc-windows-msvc` via `rustup`)
- Visual Studio C++ Build Tools (Desktop development with C++ & Windows 10/11 SDK)

### Build, Test & Package

```powershell
cargo test -j 1
cargo build --release -j 2
```

To run all tests, build the release binary (`target\release\knu_diagnosis.exe`), and package the standalone installer (`target\release\KNUDiagnosis-Setup-0.1.0.exe`):

```powershell
.\installer\build_release.ps1
```

## License

MIT License — Copyright (c) 2026 riad1666
