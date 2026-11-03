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

まだコード署名をしていないので、初回は OS の警告が出ます。

- **Windows**: 「Windows によって PC が保護されました」と出たら、「詳細情報」→「実行」
- **macOS**: 開けないと出たら、「システム設定」→「プライバシーとセキュリティ」の下にある「このまま開く」。初回にマイクの使用を許可する
- **Linux**: API キーの保存に Secret Service（GNOME キーリングや KWallet）を使う。ない環境では環境変数 `OPENAI_API_KEY` で渡す

## リリースの作り方

`Cargo.toml` の `version` を上げてコミットし、同じ版のタグ（例: `v0.2.0`）を push すると、GitHub Actions が 3 つの OS のインストーラーを作って下書きのリリースに載せます。中身を確かめてから GitHub で公開します。

## 開発

```bash
mise install
pnpm --dir frontend install
mise run dev
```

## ライセンス

[PolyForm Strict License 1.0.0](LICENSE.md)。ソースは公開していますが、改変や再配布はできません。個人の利用はできます（詳しくは本文を見てください）。
