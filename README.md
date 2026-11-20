<div align="center">

<a href="https://jimaku.deadlock.jp"><img src="web/src/assets/og.jpg" alt="デドロ字幕 — 「ドアマンのカートはディスペルで対策できます。」の字幕と英訳" width="720"></a>

# デドロ配信を、世界に。

Deadlock に特化した、配信者向けの日本語字幕＆翻訳字幕アプリ（非公式）

[![最新版](https://img.shields.io/github/v/release/Chiji1108/deadlock-jimaku?label=%E6%9C%80%E6%96%B0%E7%89%88&color=ffcf33&labelColor=12110e)](https://github.com/Chiji1108/deadlock-jimaku/releases/latest)
[![ダウンロード数](https://img.shields.io/github/downloads/Chiji1108/deadlock-jimaku/total?label=%E3%83%80%E3%82%A6%E3%83%B3%E3%83%AD%E3%83%BC%E3%83%89&color=ffcf33&labelColor=12110e)](https://github.com/Chiji1108/deadlock-jimaku/releases)
[![Discord](https://img.shields.io/badge/Discord-%E5%8F%82%E5%8A%A0%E3%81%99%E3%82%8B-5865F2?logo=discord&logoColor=white&labelColor=12110e)](https://discord.gg/QdVFzMBBD2)
[![サイト](https://img.shields.io/badge/jimaku.deadlock.jp-%E3%82%B5%E3%82%A4%E3%83%88-ffcf33?labelColor=12110e)](https://jimaku.deadlock.jp)

<br>

[![Windows 版をダウンロード](https://img.shields.io/badge/Windows-%E3%83%80%E3%82%A6%E3%83%B3%E3%83%AD%E3%83%BC%E3%83%89-0078D4?style=for-the-badge)](https://github.com/Chiji1108/deadlock-jimaku/releases/latest/download/deadlock-jimaku_x64-setup.exe)
[![Mac 版をダウンロード](https://img.shields.io/badge/macOS-%E3%83%80%E3%82%A6%E3%83%B3%E3%83%AD%E3%83%BC%E3%83%89-000000?style=for-the-badge&logo=apple&logoColor=white)](https://github.com/Chiji1108/deadlock-jimaku/releases/latest/download/deadlock-jimaku_universal.dmg)
[![Linux 版をダウンロード](https://img.shields.io/badge/Linux-%E3%83%80%E3%82%A6%E3%83%B3%E3%83%AD%E3%83%BC%E3%83%89-FCC624?style=for-the-badge&logo=linux&logoColor=black)](https://github.com/Chiji1108/deadlock-jimaku/releases/latest/download/deadlock-jimaku_amd64.AppImage)

<sub>Windows 10 / 11 ・ macOS 11 以降（Apple シリコン・Intel）・ Linux（AppImage ／ <a href="https://github.com/Chiji1108/deadlock-jimaku/releases/latest/download/deadlock-jimaku_amd64.deb">.deb</a> ／ <a href="https://github.com/Chiji1108/deadlock-jimaku/releases/latest/download/deadlock-jimaku.x86_64.rpm">.rpm</a>）</sub>

</div>

---

## できること

- 💬 **字幕** — マイクの声をリアルタイムに字幕にし、ふりがなと翻訳（最大 4 言語）を付ける。ヒーロー・アイテム・スキルの名前はアイコン付きで強調する
- 🏆 **ランク** — ランクのアイコン・名前・ポイントのゲージ
- 🃏 **アイテム説明** — 字幕でアイテム名が出たら、その説明をゲーム内と同じカードで出す

## はじめかた

1. **ダウンロード** — 上のボタンから、使う PC に合ったものを入れます。2 PC 配信なら、配信用の PC（マイクと OBS がある方）に入れます
2. **OBS に追加** — アプリの画面のボタンを OBS にドラッグするだけです
3. **字幕を始める** — まずは無料の Chrome のモード（既定）で、キーなしで始まります

> [!NOTE]
> 既定のモードは途中の字幕を Chrome で作るので、**Google Chrome が必要**です。字幕を始めると小さな Chrome のウィンドウが開きます（クラウドとローカルのモードは Chrome を使いません）。

<details>
<summary><b>初めて開くとき</b></summary>

macOS 版は Apple の署名と公証を済ませています。Windows 版はまだコード署名をしていないので、初回は警告が出ます。

- **Windows**: 「Windows によって PC が保護されました」と出たら、「詳細情報」→「実行」
- **macOS**: dmg を開いて、アプリを「アプリケーション」にドラッグする。初回にマイクの使用を許可する
- **Linux**: AppImage は「プログラムとして実行」を許可してから開く（`chmod +x` でも可）。API キーの保存に Secret Service（GNOME キーリングや KWallet）を使う

</details>

## モード

字幕の作り方はモードから選べます。設定はアプリの操作パネルから行います。

| モード | 料金 | ひとこと |
|---|---|---|
| **Chrome**（既定） | 無料 | Chrome の音声認識と翻訳だけを使う。サーバーを使わない |
| **おすすめ** | ベータ期間中は無料 | 確定の字幕・用語・翻訳を、デドロ字幕のサーバーを通して高精度に作る |
| **Chrome+** | 無料 | Chrome の中の AI で用語を判定する。VRAM 8 GB 以上推奨 |

<details>
<summary><b>その他のモード</b></summary>

| モード | 料金 | ひとこと |
|---|---|---|
| **最高精度** | OpenAI の API キー | 用語の判定と文字起こしがいちばん正確 |
| **クラウド** | ベータ期間中は無料 | Chrome を開かず、音声を Google に送らない。使用メモリ最小 |
| **ローカル** | 無料 | PC の中の AI だけで作り、外に送らない。VRAM 12 GB 以上推奨 |

</details>

「モード」のタブで、工程（途中の字幕・確定の文字起こし・用語・翻訳・ふりがな）ごとに作り方を変えることもできます。

---

<div align="center">

**🛠️ ここから下は開発する人向けです**

</div>

## しくみ

開発する人のための地図です。理由は [DESIGN.md](DESIGN.md) を見てください（モードは 9.6、ローカルの AI は 9.8〜9.11 と 9.14、数字と句読点は 9.15、開発者のサーバーと LP は 17 章）。

字幕は 5 つの工程で作ります。工程ごとに何を使うかは設定の `engines` にあり、モードはその組み合わせに付けた名前です（`Engines::MODES`。`crates/core/src/settings.rs`）。操作パネルでモードを選ぶと 5 つがまとめて決まり、「モード」のタブで工程ごとに変えると「カスタム」になります。

| 工程 | おすすめ | Chrome（既定） | Chrome+ | 最高精度 | クラウド | ローカル |
|---|---|---|---|---|---|---|
| 途中の字幕 | Chrome | Chrome | Chrome | Chrome | Qwen-Audio-3.1-ASR-Flash-Streaming | Haas-ja |
| 確定の文字起こし | Qwen-Audio-3.1-ASR-Flash | Chrome | Chrome | GPT-Transcribe | Qwen-Audio-3.1-ASR-Flash | Whisper large-v3-turbo |
| 用語 | Perplexity Decider | 辞書の一致 | Gemini Nano | GPT-6.1 Terra | Perplexity Decider | Gemma 4 E4B |
| 翻訳 | Hy-MT2 | Chrome Translator | Chrome Translator | GPT-6.1 Terra | Hy-MT2 | Gemma 4 E4B |
| ふりがな | 辞書 + Perplexity Decider | 辞書 | 辞書 | 辞書 + GPT-6.1 Terra | 辞書 + Perplexity Decider | 辞書 |
| 要るもの | なし（ベータ期間中） | なし | なし | OpenAI の API キー | なし（ベータ期間中） | なし |
| 声と文字の送り先 | Google、開発者のサーバー | Google | Google | Google、OpenAI | 開発者のサーバー | PC から出ない |

- 途中の字幕が Chrome のときは、話している途中から訳も出ます（Chrome Translator）。途中の字幕には、辞書の読みと、辞書の一致で見つけた用語が付きます。
- 翻訳先がない、翻訳だけを見せる、ふりがなやデッドロックモードを切る、話す言語が日本語でない（読みの辞書と用語の一致は日本語だけ）ときは、その工程を動かしません。設定と `engines` から、実際に何を動かすかを `Settings::plan` が決めます。
- 作れない組み合わせは `Engines::sanitized` が直します。確定の文字起こしが Chrome なら途中の字幕も Chrome（発話の区切りを Chrome が決める）、Chrome の翻訳は途中の字幕が Chrome のときだけです。翻訳を GPT にしたときは、用語も同じ問い合わせで判定します（訳文の名前を用語から作るので）。

**工程ごとに選べるもの。** 表のほかにも選べます。「おすすめ」と「クラウド」のモードのほかは、その会社の自分の API キーが要ります（PC の中で動くものは要りません）。

- **途中の字幕と確定の文字起こし**（`stt/`、`local.rs`、`whisper.rs`、`reazon.rs`）: Chrome、OpenAI（GPT-Live-Transcribe、GPT-Transcribe）、xAI の Grok Voice Transcribe、Qwen-Audio-3.1-ASR-Flash（DESIGN.md 3.4）、OpenRouter の MAI-Transcribe 2、PC の中の Haas-ja（speech.cpp）と Whisper（whisper.cpp。ヒーローの名前を教える）。確定だけなら、PC の中の ReazonSpeech k2（補助プログラム `crates/sherpa`。辞書の用語をホットワードで教える。DESIGN.md 9.14）も選べます。
- **用語**（`terms.rs`、`decide.rs`）: 辞書の一致か、PC の中で出した候補を判定モデルに確かめさせます。判定モデルは、Chrome の中の Gemini Nano、PC の中の Gemma 4 E4B、OpenAI の Decisions API、TypeSafe AI の Jev、OpenRouter の Drex Max・Microsoft Decision 1・Perplexity Decider（DESIGN.md 9.7）。
- **翻訳**（`mt.rs`、`enrich.rs`）: 用語を英語名に置き換えた文を、Chrome Translator、PC の中の Gemma 4 E4B、翻訳専門のモデル（Qwen-MT-Flash、OpenRouter の Hy-MT2）に渡します。GPT-6.1 Terra は、用語と翻訳を 1 回の問い合わせで作ります。
- **ふりがな**（`reading.rs`、`furigana.rs`）: 辞書の読み（vibrato と話し言葉の UniDic）をそのまま使うか、判定モデルか GPT に直させます。

確定の文には、どの組み合わせでも `finish.rs` が工程ごとに用語・ふりがな・翻訳を付け、1 つの字幕にまとめます。その前に、日本語の文は `tidy.rs` が数字と句読点を整えます（DESIGN.md 9.15）。数のすぐ後ろに数える言葉があれば算用数字にし（「三十五秒」→「35秒」）、句読点を書かない文字起こし（Chrome、ReazonSpeech k2）の文には、話の間の空白から「、」と「。」を付けます。ふりがなと同じ形態素解析の辞書を使い、モデルは使いません。

```mermaid
flowchart LR
  subgraph pc["配信 PC"]
    obs["OBS<br/>オーバーレイ<br/>ドック"]
    subgraph app["Tauri アプリ（app/）"]
      panel["操作パネル<br/>control.html"]
      core["中核（crates/core）<br/>字幕の処理、Store<br/>localhost:4510 の /api、/ws、/overlay/*"]
      shell["トレイ、通知、ホットキー<br/>自動更新"]
    end
    tab["Chrome のウィンドウ<br/>/chrome<br/>音声認識、翻訳、Gemini Nano"]
    local["ローカルの AI<br/>speech.cpp、whisper.cpp、llama.cpp"]
  end
  worker["Cloudflare Worker（server/）<br/>ベータの無料の提供"]
  openai["OpenAI"]
  xai["xAI"]
  qwen["Qwen"]
  others["OpenRouter<br/>TypeSafe AI"]
  google["Google の音声認識"]
  deadlockapi["deadlock-api"]
  releases["GitHub Releases"]
  lp["LP（web/）<br/>jimaku.deadlock.jp"]

  obs --> core
  panel --> core
  tab <-- "文と訳" --> core
  core -- "起動、閉じる" --> tab
  core <-- "音声、文字" --> local
  tab -- "声" --> google
  core -- "ベータのコード" --> worker
  worker -- "開発者のキー" --> qwen
  worker -- "開発者のキー" --> others
  core -- "自分の API キー" --> openai
  core -- "自分の API キー" --> xai
  core -- "自分の API キー" --> qwen
  core -- "自分の API キー" --> others
  core -- "ランク、辞書" --> deadlockapi
  shell -- "新しい版を確認" --> releases
  lp -- "ダウンロードのリンク" --> releases
```

- 操作パネルは Tauri のウィンドウでも OBS のドックでも同じ画面で、`/api` と `/ws` だけを使います。
- 途中の字幕が Chrome の組み合わせでは、アプリが専用のプロファイルの Chrome を起動します。音声認識と翻訳は Chrome が行い、ふりがなと辞書の一致は中核の中で行います。
- ローカルの AI（Haas-ja、Whisper、Gemma 4 E4B）は、選んだときに初めてダウンロードし、別のプロセスで立てて、OpenAI と同じ形の口で頼みます。字幕を止めると終わります。
- OpenAI と xAI、TypeSafe AI には、自分の API キーで直接送ります。Qwen と OpenRouter は、ベータのコードがあれば Worker を通し（開発者のキー。「おすすめ」と「クラウド」のモード）、なければ自分の API キーで直接送ります。Worker は会社ごとに同じパスで中継するので、アプリは送り先を替えるだけです（`server/src/beta.ts`。DESIGN.md 17.2）。

### 声と文字の流れ

代表の 3 つのモードの、処理の順です。黄色は PC の外です。字幕は `/ws` でオーバーレイと操作パネルに届きます。

**おすすめ。** 途中の字幕は Chrome で作ります。発話が終わると、アプリのマイクの声を開発者のサーバーに送り、確定の文に用語・翻訳・ふりがなを付けます。Chrome にも同じマイクを渡します（渡せない環境では Chrome の既定のマイクを使い、操作パネルに出します）。「最高精度」も同じ形で、送り先が OpenAI（自分の API キー）になります。「クラウド」は、途中の字幕も開発者のサーバーを通して作り、Chrome を開きません。

```mermaid
flowchart LR
  mic["マイク"] --> core["中核<br/>発話を区切る<br/>読み、用語の候補"]
  mic --> tab["Chrome のウィンドウ<br/>/chrome<br/>途中の翻訳"]
  tab -- "声" --> google["Google の音声認識"]
  google -- "途中の文" --> tab
  tab <-- "/api/chrome/ws" --> core
  core -- "区切った音声" --> worker["開発者のサーバー"]
  worker --> stt["Qwen<br/>確定の文字起こし"]
  stt --> words["OpenRouter<br/>用語、翻訳、ふりがな"]
  core --> ws["/ws"]
  words --> ws
  classDef cloud fill:#ffcf33,stroke:#12110e,color:#12110e
  class google,worker,stt,words cloud
```

**Chrome。** 音声認識も翻訳も Chrome で行い、声は Chrome を通じて Google に送られます。読みと用語は中核で付け、ほかには何も送りません。「Chrome+」は、用語の候補を Chrome の中の Gemini Nano に確かめさせます。

```mermaid
flowchart LR
  mic["マイク"] --> tab["Chrome のウィンドウ<br/>/chrome<br/>翻訳"]
  tab -- "声" --> google["Google の音声認識"]
  google -- "途中と確定の文" --> tab
  tab <-- "/api/chrome/ws" --> core["中核<br/>読み、用語"]
  core --> ws["/ws"]
  classDef cloud fill:#ffcf33,stroke:#12110e,color:#12110e
  class google cloud
```

**ローカル。** 声も文字も PC から出しません。Chrome も開きません。

```mermaid
flowchart LR
  mic["マイク"] --> core["中核<br/>発話を区切る<br/>読み、用語の候補"]
  core -- "話している間の音声" --> live["speech.cpp（Haas-ja）<br/>途中の字幕"]
  core -- "区切った音声" --> stt["whisper.cpp（Whisper）<br/>確定の文字起こし"]
  stt --> words["llama.cpp（Gemma 4 E4B）<br/>用語、翻訳"]
  live --> ws["/ws"]
  words --> ws
```

読みと辞書の一致は話す言語が日本語のときだけ動きます。ほかの言語でも Chrome は認識するので、字幕は出ます。

## リリースの作り方

使う人に関係のある変更があれば、`CHANGELOG.md` にその版の内容を書きます（更新したあとの最初の起動で、アプリがこの内容を出します。プレリリースの間は、本リリースの版の見出しに書き足します）。`Cargo.toml` の `version` を上げてコミットし、同じ版のタグ（例: `v0.2.0`。`-` の付いた `v0.2.0-rc.1` はプレリリース）を push すると、GitHub Actions が 3 つの OS のインストーラーを作って下書きのリリースに載せます（`.github/workflows/release.yml`）。プレリリースでは `.rpm` を作りません。中身を確かめてから GitHub で公開します。PR と main でも同じビルドが走り（`build.yml`）、作ったインストーラーは Actions の成果物から落とせます。

macOS 版の署名と公証には、リポジトリの Actions の Secrets に次を入れておきます（リリースのときだけ使い、PR と main ではアドホック署名）。

| Secret | 中身 |
|---|---|
| `APPLE_CERTIFICATE` | Developer ID Application 証明書（1 つだけ）の `.p12` を base64 にしたもの |
| `APPLE_CERTIFICATE_PASSWORD` | `.p12` を書き出したときのパスワード |
| `APPLE_API_ISSUER` | App Store Connect の API キーの Issuer ID |
| `APPLE_API_KEY` | 同じく Key ID |
| `APPLE_API_KEY_P8` | 同じく `.p8` ファイルの中身 |

自動更新の署名には、`TAURI_SIGNING_PRIVATE_KEY` と `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` も入れておきます。

## 開発

```bash
mise install
pnpm install
mise run dev
```

整形・リント・型チェック・テストは `mise run check` でまとめて実行します。

使っているアプリを動かしたまま、別の場所で起動して試せます。`DEADLOCK_JIMAKU_DATA_DIR`（設定と辞書を置くフォルダ）と `DEADLOCK_JIMAKU_PORT`（ポート）を付けます。フォルダを替えると、API キーも別の名前で持つので、使っているアプリのキーは読みません。

```bash
pnpm --dir frontend build
DEADLOCK_JIMAKU_DATA_DIR=/tmp/jimaku-dev DEADLOCK_JIMAKU_PORT=4599 cargo run -p overlays-core --bin overlays-headless
```

操作パネルは `http://127.0.0.1:4599/` で開きます。

報告や提案に AI を使うときは、[AI の利用について](AI_POLICY.md)を読んでください。

## ライセンス

[PolyForm Strict License 1.0.0](LICENSE.md)。ソースは公開していますが、改変や再配布はできません。個人の利用はできます（詳しくは本文を見てください）。

<sub>参考にしたプロジェクト: [音声認識字幕ちゃん](https://github.com/sayonari/jimakuChan)（sayonari さん。Chrome の音声認識の扱いの設計）、[ゆかりねっとコネクター NEO](https://nmori.github.io/yncneo-Docs/)（Chrome の音声認識を選ぶきっかけ）、[Mojicast](https://github.com/ishiki-emo/mojicast)（ishiki-emo さん。ローカルの音声認識）。ありがとうございます。</sub>
