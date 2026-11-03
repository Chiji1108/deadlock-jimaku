# Deadlock Stream Overlays

Deadlock の配信者向けの OBS オーバーレイ。

- **字幕**: マイクの声をリアルタイムに字幕にし、ふりがなと翻訳（最大 4 言語）を付ける。ヒーロー・アイテム・スキルの名前はアイコン付きで強調する
- **ランク**: ランクのアイコン・名前・ポイントのゲージ
- **アイテム説明**: 字幕でアイテム名が出たら、その説明をゲーム内と同じカードで出す

音声認識と翻訳には OpenAI の API キーが必要です。設定はアプリの操作パネルから行います。

設計は [DESIGN.md](DESIGN.md) を見てください。

## インストール

[Releases](https://github.com/Chiji1108/deadlock-stream-overlays/releases) から、使う PC に合ったものをダウンロードします。2 PC 配信なら、配信用の PC（マイクと OBS がある方）に入れます。

| OS | ファイル |
|---|---|
| Windows 10 / 11 | `..._x64-setup.exe` |
| macOS 11 以降（Apple シリコン・Intel） | `..._universal.dmg` |
| Linux | `.AppImage`（どのディストリビューションでも）、`.deb`（Ubuntu など）、`.rpm`（Fedora など） |

macOS 版は Apple の署名と公証を済ませています。Windows 版はまだコード署名をしていないので、初回は警告が出ます。

- **Windows**: 「Windows によって PC が保護されました」と出たら、「詳細情報」→「実行」
- **macOS**: 初回にマイクの使用を許可する
- **Linux**: API キーの保存に Secret Service（GNOME キーリングや KWallet）を使う。ない環境では環境変数 `OPENAI_API_KEY` で渡す

## リリースの作り方

`Cargo.toml` の `version` を上げてコミットし、同じ版のタグ（例: `v0.2.0`。`-` の付いた `v0.2.0-rc.1` はプレリリース）を push すると、GitHub Actions が 3 つの OS のインストーラーを作って下書きのリリースに載せます（`.github/workflows/release.yml`）。中身を確かめてから GitHub で公開します。PR と main でも同じビルドが走り（`build.yml`）、作ったインストーラーは Actions の成果物から落とせます。

macOS 版の署名と公証には、リポジトリの Actions の Secrets に次を入れておきます（リリースのときだけ使い、PR と main ではアドホック署名）。

| Secret | 中身 |
|---|---|
| `APPLE_CERTIFICATE` | Developer ID Application 証明書（1 つだけ）の `.p12` を base64 にしたもの |
| `APPLE_CERTIFICATE_PASSWORD` | `.p12` を書き出したときのパスワード |
| `APPLE_API_ISSUER` | App Store Connect の API キーの Issuer ID |
| `APPLE_API_KEY` | 同じく Key ID |
| `APPLE_API_KEY_P8` | 同じく `.p8` ファイルの中身 |

## 開発

```bash
mise install
pnpm --dir frontend install
mise run dev
```

## ライセンス

[PolyForm Strict License 1.0.0](LICENSE.md)。ソースは公開していますが、改変や再配布はできません。個人の利用はできます（詳しくは本文を見てください）。
