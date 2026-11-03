# デドロ字幕 設計書

デドロ字幕。Deadlock 配信者向けの OBS オーバーレイツール。

- **字幕**: 配信者の声をリアルタイムで字幕にする。ふりがな付きの日本語と、翻訳を表示する。ヒーロー・アイテム・スキル名はゲームアイコン付きで強調する。
- **ランク**: 配信者の現在のランクを表示する。

## 目的

- 無音でも配信を楽しめるようにする
- 日本語を学んでいる海外の視聴者の教材になる（日本語には常にふりがなを付ける）
- ゲーム用語は各言語の公式名で表示し、アイコンで一目でわかるようにする

## スコープ

- 字幕にするのは**配信者のマイクだけ**。ゲーム内 VC や Discord の声は扱わない（理由は「14. 将来の拡張案」を参照）。
- 翻訳先は deadlock-api が対応している 29 言語から選ぶ。既定は英語のみ。

## 前提

- 配信 PC は Windows（Deadlock・OBS・本ツールを同じマシンで動かす）
- ヘッドホンを使う（マイクにゲーム音が回り込まない）
- 音声認識: OpenAI（GPT-Live-Transcribe と GPT-Transcribe。3 章）
- 翻訳と辞書の漢字名の読み: OpenAI GPT-6.1 Terra（推論なし。9.3）
- ふりがな: GPT-6.1 Terra（文ごと。答えは確かめてから使う）、固有名詞: 辞書との照合（9 章）
- ゲームデータ: [deadlock-api](https://api.deadlock-api.com/docs)（MIT）

---

## 1. 全体アーキテクチャ

```
┌──────────────────── Tauri v2 アプリ（Windows 常駐・トレイ）────────────────────┐
│                                                                                │
│  mic ──▶ subtitles pipeline ──┐                                                │
│                               ├──▶ store + broadcast ──▶ axum (localhost:PORT) │
│  assets (glossary / icons) ───┤                          ├─ GET  /overlay/*    │
│  rank (polling) ──────────────┘                          ├─ GET  /assets/*     │
│                                                          ├─ WS   /ws           │
│                                                          └─ REST /api/*        │
│                                                                                │
│  Tauri ウィンドウ = control.html（/api と /ws を使うだけ）                     │
└────────────────────────────────────────────────────────────────────────────────┘
        ▲                                         ▲
        │ OBS カスタムブラウザドック              │ OBS ブラウザソース
        │ （control.html をそのまま表示できる）   │ （/overlay/subtitles, /overlay/rank）
```

### 方針

- **API は 1 つ**: 操作パネルは Tauri IPC を使わず、`localhost` の `/api` と `/ws` を使う。同じ画面が Tauri のウィンドウでも OBS のドックでも動く。Tauri 側で扱うのは、トレイ常駐・自動更新・API キーの保存だけ。
- **状態はサーバーが持ち、画面は表示に徹する**: OBS でソースを再読み込みしても、接続直後のスナップショットで表示が元に戻る。
- **機能はモジュール、表示はトピック**: オーバーレイは必要なトピック（`subtitle`, `rank`）だけを購読する。機能を足しても中核の構造は変わらない。
- **言語はデータ**: 言語ごとの分岐をコードに書かない。言語の対応表（「6. 言語」）に 1 行足せば対応言語が増える。

## 2. 技術スタック

| 層 | 採用 | 理由 |
|---|---|---|
| デスクトップ | **Tauri v2** | インストーラー（.msi）、自動更新、トレイ常駐、Rust の中核 |
| HTTP / WS | **axum** | Tauri と同じ tokio ランタイム、WS が標準で使える、tower エコシステム |
| 静的配信 | `rust-embed`（本番）/ Vite dev server（開発） | 本番は 1 つの exe に同梱 |
| 型共有 | **ts-rs** | Rust の型から TS の型を生成し、フロントとの型ずれをなくす |
| マイク入力 | `cpal`（WASAPI） | |
| VAD | **earshot**（純 Rust、16ms フレーム） | 字幕の区切りを決める（3.1）。Silero と違って ONNX Runtime が要らず、exe に 100KB 程度しか足さない |
| リサンプル | `rubato` | 16kHz mono に揃える |
| OpenAI | `reqwest` + `tokio-tungstenite` | 公式の Rust SDK がないので直接呼ぶ |
| API キー | `keyring`（v1） | Windows の資格情報マネージャー / macOS のキーチェーン |
| 設定 | JSON（アプリの設定ディレクトリ） | |
| フロント | **Svelte 5 (runes) + Vite + TypeScript** | 下記 |
| スタイル | オーバーレイ: 素の CSS + CSS 変数 / 操作パネル: shadcn-svelte（Tailwind CSS v4） | オーバーレイは OBS の「カスタム CSS」で見た目を上書きできる |

### フロントに Svelte 5 を選ぶ理由

- このオーバーレイの中心は「行が現れる・中身が変わる・押し上がる・消える」というアニメーション。Svelte は `transition:` と `animate:` が標準で入っていて、追加のライブラリなしで書ける。
- partial 字幕は頻繁に更新される。runes の細かい単位の反応性で、変わった部分だけを描き直す。
- ランタイムが小さく、OBS の CEF でも軽い。
- SvelteKit は使わない。Vite のマルチエントリで `control.html` / `overlay/subtitles.html` / `overlay/rank.html` を作る（SSR もルーターも要らない）。

### 開発時

- Vite dev server で HMR を使う。`/ws` `/api` `/assets` は axum にプロキシする。
- 開発中も OBS から Vite の URL を指せば、HMR しながらオーバーレイを調整できる。
- ツールのバージョンは `mise.toml` で固定する（Rust / Node / pnpm）。

| コマンド | 内容 |
|---|---|
| `mise run dev` | Tauri アプリを開発モードで起動（Vite の HMR 付き） |
| `mise run headless` | Tauri なしで中核だけ起動し、ダミー字幕を流し続ける（`--demo`。`http://127.0.0.1:4510/overlay/subtitles`） |
| `mise run check` | oxfmt / oxlint / svelte-check / rustfmt / clippy / cargo test |

アプリは環境変数の API キーを読まない（開発中も操作パネルで保存する）。`experiments/` のスクリプト用のキーは、リポジトリ直下の `.env.local`（`OPENAI_API_KEY=…`。コミットしない）に書けば、mise が環境変数として渡す（`mise run` で起動したアプリにも入るが、アプリは読まない）。

ヘッドレスの `overlays-headless` も Tauri アプリと同じ `settings.json` と API キーを使い、操作パネル（`http://127.0.0.1:4510/`）で設定する。引数は開発用の `--demo` と、マイクの代わりに録音を実時間で流す `--input-file PATH.wav` だけ。

- Git フック（**lefthook**）: commit 時に変更ファイルへ oxfmt / oxlint / cargo fmt、push 時に `mise run check`。clone 後に `mise exec -- lefthook install` を一度実行する。
- 整形とリント: フロントは **oxfmt**（`svelte: true` で `.svelte` も整形）と **oxlint**、Rust は rustfmt と clippy。
- TypeScript は 6 系に固定する（svelte-check がまだ 7 系に対応していない）。
- `frontend/src/generated/` は ts-rs の出力（`cargo test -p overlays-core` で更新）。手で編集しない。

## 3. 音声認識: OpenAI（2026-11 時点のドキュメントで確認）

| 用途 | モデル ID | 料金 | 備考 |
|---|---|---|---|
| partial | `gpt-live-transcribe` | $0.017/分 | Realtime の文字起こしセッション（WebSocket）。サーバーは発話を区切らない |
| final | `gpt-transcribe` | $0.0045/分 | `/v1/audio/transcriptions`。精度が最も高い |
| （翻訳・読みも OpenAI `gpt-6.1-terra`。9.3） | — | — | キーは OpenAI の 1 つで済む |

モデル ID はすべて設定で持つ（`models.realtime` / `models.batch`）。

**Mistral Voxtral から切り替えた理由**（2026-11、`experiments/stt_eval*.py`。用語のヒントあり）:

| | sample01 | sample02 |
|---|---|---|
| partial: Voxtral Mini Transcribe Realtime 3 | 4.8% | 12.7%（拡マガ → 隠間が） |
| partial: GPT-Live-Transcribe | 4.0% | 4.9% |
| final: Voxtral Small Transcribe 3 | 5.6% | 7.8%（取りたい → 撮りたい） |
| final: GPT-Transcribe | 0.8% | 3.9% |

（文字誤り率）。速さはアプリで同じ録音を流して測った: 話し始めから最初の partial まで OpenAI 約 0.9〜1.1 秒 / Voxtral 約 1.6 秒、最後の partial から final まで OpenAI 約 1.2〜1.7 秒 / Voxtral 約 0.7〜1.0 秒。単価は約 2 倍だが、話していない間は送らなくなったので、1 時間あたりの費用はほぼ変わらない（3.3）。

**ElevenLabs Scribe v2 も比べたが、切り替えない**（2026-11、`experiments/stt_eval_elevenlabs.py`）:

| | sample01 | sample02 |
|---|---|---|
| Scribe v2 Realtime（確定した文をつなげたもの。用語なし / ヒーロー名 44 件） | 0.8% / 0.8% | 9.8% / 7.8%（「い--」「うん」など言いよどみまで書く） |
| 同（ヒーロー名 44 件 + `no_verbatim`） | 6.5%（「えーね」→「いいね」） | 7.8% |
| final: Scribe v2（用語なし / 634 件） | 0.8〜1.6% / 1.6% | 3.9% / 5.9% |

- final は GPT-Transcribe と同じか、用語を渡すと少し悪い（「だな」→「だなあ」、「まあ」を落とす）。同じ設定でも回によって「だな」「だなー」と揺れる
- Realtime は用語が 50 件・20 文字までで、別名（拡マガなど）を渡しきれない。区切りはサーバーの VAD（無音 0.75 秒）と最後の手動 commit で、アプリの区切りとは同じでない（用語なし / ありで区切りの数も違った）
- 最初の partial が出るまで（接続から）はどちらも約 2.5 秒で差がない

### 3.1 Realtime（GPT-Live-Transcribe）

- URL: `wss://api.openai.com/v1/realtime?intent=transcription`。認証は `Authorization: Bearer <key>` ヘッダー（ブラウザの WebSocket では付けられないので、Rust の中核から接続する）
- 流れ: 接続 → `session.created` → `session.update` → `session.updated` → 発話を開いたら `input_audio_buffer.append`（24kHz の PCM を base64。100ms ずつ）を送り始める → **発話を閉じたら `input_audio_buffer.commit`**（100ms 未満なら commit できないので `input_audio_buffer.clear`）
- 話していない間の音声は送らない（リアルタイムは送った音声の長さで課金される）。発話を開く前の 0.3 秒だけ手元に持ち、開いたら先に送る（VAD が開くのは話し始めより少し遅い）
- 16kHz → 24kHz はちょうど 3/2 倍なので、ため込まない線形補間で変換する（発話の終わりまでその場で送れる）
- 接続と設定には 10 秒の上限を付ける。接続している間・切れている間に溜まった音声は送らず、今の発話の続きから送る
- `turn_detection` は使わない（`null`。このモデルはサーバー側の VAD に対応していない）。区切りはローカルの VAD で決める（下記）

```json
{
  "type": "session.update",
  "session": {
    "type": "transcription",
    "audio": { "input": {
      "format": { "type": "audio/pcm", "rate": 24000 },
      "transcription": {
        "model": "gpt-live-transcribe",
        "languages": ["ja"],
        "prompt": "Live commentary by a streamer playing the video game Deadlock (Valve).",
        "keywords": ["インファーナス", "基礎拡張マガジン", "..."],
        "delay": "low"
      },
      "turn_detection": null
    }}
  }
}
```

| フィールド | 値 |
|---|---|
| `delay` | `minimal` / `low` / `medium` / `high` / `xhigh`。低いほど早く、高いほど正確（設定の `realtimeDelay`。既定 `low`） |
| `languages` | 話す言語（ISO 639-1）。単数の `language` と一緒には送らない |
| `prompt` | 録音の説明 |
| `keywords` | 出てきそうな用語（8.4）。`<` `>` と改行を含めない |

サーバーからのイベント:

| type | 内容 |
|---|---|
| `conversation.item.input_audio_transcription.delta` | `item_id`, `delta`。**commit の前から流れる**。過去のテキストは訂正されない（追記のみ） |
| `input_audio_buffer.committed` | `item_id`。commit した順に届く |
| `conversation.item.input_audio_transcription.completed` | `transcript`（その item の全文。final はバッチで取り直すので使わない） |
| `error` | 1 つの要求の失敗（短すぎる commit など）。セッションは続く |

**commit と item**: 1 回の commit までの音声が 1 つの item になり、次の音声は新しい item になる。delta には最初から `item_id` が付くので、item と Segment を commit の順で 1 対 1 に結びつける（7.2）。対応は音声認識の接続の中だけで持ち（`stt.rs`）、字幕の流れには Segment の ID 付きで delta を返す。張り直したら対応も作り直すので、イベントが 1 つ欠けても以降がずれない。確認の前に初めて見る item は推測で付けるが、commit の確認（`input_audio_buffer.committed`）で commit した順の Segment に付け直すので、推測が違っていても直る（ずれた数文字も final で置き換わる）。commit には `event_id` に Segment の ID を付け、断られたら（`error.event_id`）その音声を消して Segment の対応も消す。文字が届き終わった item（`completed`）の対応は消す。入力が終わったら最後の発話を閉じて終わる（確定はバッチで取る）。

**発話の区切り方**: 区切りはローカルの VAD（earshot）で決める。会話の「話す番」を判定するターン検出は字幕の区切りと目的が違い、話し続ける配信者では区切りがいつまでも来ない（Voxtral の検証で、間を挟みながら 22 秒話した音声が 1 ターンになった）。

- 無音が `minSilenceMs`（既定 750ms。操作パネルで すぐ 500 / ふつう 750 / つなぐ 1000 か、「数値で指定」のダイアログで 200〜2000 を選ぶ）続いたら Segment を閉じる。話している途中は partial が出続けるので、長くしても字幕が出始めるのは遅れない（確定と翻訳が遅れる）
- 長さが `maxSegmentSeconds`（既定 8 秒）を超えたら、次の短い無音で強制的に閉じる（字幕が長くなりすぎないように）
- 音声は手元にあるので、区切りの位置が正確にわかり、追加のコストもかからない

### 3.2 GPT-Transcribe（バッチ）

- `/v1/audio/transcriptions` に multipart で `model`, `file`（Segment の区間の WAV。16kHz）, `prompt`, `languages[]`, `keywords[]`（realtime と同じもの）を送る

### 3.3 1 時間あたりのコスト見積もり

| 項目 | 見積もり |
|---|---|
| Realtime（話している間だけ送る。仮に 40% + 発話ごとの前後） | 約 26 分 × $0.017 = **約 $0.44** |
| GPT-Transcribe（発話区間のみ） | 約 26 分 × $0.0045 = **約 $0.12** |
| 翻訳（GPT-6.1 Terra。1 時間に約 600 発話） | 翻訳先 1 言語で **約 $0.3**、1 言語増えるごとに **+約 $0.3** |
| ふりがな（GPT-6.1 Terra。漢字を含む発話だけ） | **約 $0.3** |
| 合計（翻訳先が英語のみ、ふりがなあり） | **約 $1.2/時間**（話し続けると約 $2。下の実測） |

アプリで数えた実測（sample02、29 秒・5 発話）: 途中の字幕 $0.0058（21 秒）、確定の字幕 $0.0015（20 秒）、翻訳 $0.0024（1,771 トークン）、ふりがな $0.0026（1,333 トークン）、合計 $0.012。話し続けた 1 時間に直すと、sample01 で約 $1.9、sample02 で約 $1.5。LP には「話し続けて約 $2」と書く。操作パネルの「料金」タブに出す（13 章）。

Voxtral のときはリアルタイムに無音も含めて送り続けていた（delta を届いた時刻で発話に振り分けていたので、途中を抜けなかった）。OpenAI では発話ごとに commit するので、話していない間は送らない。

## 4. リポジトリ構成

```
deadlock-stream-overlays/
├─ DESIGN.md
├─ Cargo.toml                 # Rust の workspace（crates/core と app）
├─ package.json               # pnpm の workspace（frontend・web・server）
├─ pnpm-workspace.yaml
├─ crates/
│  └─ core/                   # Tauri に依存しない。単体でヘッドレス起動・テストできる
│     └─ src/
│        ├─ audio.rs          # マイク → 16kHz mono フレーム + リングバッファ
│        ├─ stt.rs            # OpenAI の realtime / batch クライアント
│        ├─ enrich.rs         # 翻訳（OpenAI）の呼び出し・出力の検証
│        ├─ subtitles.rs      # パイプライン（Segment の状態遷移）
│        ├─ assets/           # deadlock-api → glossary + アイコンのキャッシュ
│        ├─ rank.rs           # ランクのポーリング
│        ├─ lang.rs           # 言語の対応表
│        ├─ store.rs          # 状態ストア + broadcast
│        ├─ server.rs         # axum のルーティング
│        └─ types.rs          # ts-rs で TS に出力する型
├─ app/                       # Tauri シェル（トレイ、keyring、自動更新、core の起動）
├─ data/                      # 手で管理する辞書データ（git 管理）
│  ├─ aliases.ja.json         # 日本語の略称・俗称
│  └─ readings.ja.json        # 漢字を含む名前のふりがな
└─ frontend/
   ├─ control.html
   ├─ overlay/subtitles.html
   ├─ overlay/rank.html
   └─ src/
      ├─ lib/ws.ts            # 再接続付きの WS クライアント（トピック購読）
      ├─ lib/Rich.svelte      # Rich テキストの描画（ルビ・アイコン・ハイライト）
      ├─ lib/lang.css         # 言語ごとのフォント
      └─ generated/           # ts-rs の出力（言語の対応表もここに出す）
```

## 5. 設定（`settings.rs`, `secrets.rs`, `runtime.rs`）

- 設定は `{データディレクトリ}/settings.json`（Tauri の `app_data_dir`。ヘッドレスも同じ場所）に JSON で保存する。アプリが書き直すファイルなので、コメントの書ける TOML にはしない（書き直すと消える）
  - 項目が欠けていれば既定値。壊れていれば `.broken` に退けて既定値から始める。権限などで読めないときは既定値で動き、変更はメモリにだけ当てて**保存しない**（ファイルを既定値で上書きしない。操作パネルにその旨を出す）
  - 既定値のままの詳細項目（`models` など）はファイルに書かない（既定値を変えたら、変えていない人にも届く）
  - 保存は読み込み〜書き込みをまとめて排他する（同時の変更が消えない）
  - 型は `types.rs` と同じく ts-rs で TS に出す（`Settings`, `SettingsPatch`）
- **API キーは設定に入れない**。OS の資格情報ストア（Windows の資格情報マネージャー、macOS のキーチェーン。`keyring` クレート）に保存し、画面には「保存済み / 未設定 / 読めない（理由）」だけを返す。環境変数は読まない（ほかのツールのために設定した `OPENAI_API_KEY` を黙って使うと、思わぬキーに請求が付く）
- **変更はその場で反映する**: `Store` が設定を watch チャンネルで持ち、`runtime` が「辞書・ランク・字幕」の処理を設定から起動する。設定か API キーが変わったら、**入力が変わった処理だけ**止めて張り直す（ランクの ID を変えても字幕は止めない）
  - 辞書: 言語・OpenAI のキー / ランク: アカウント・言語・間隔（アカウントが変わったら前のランクを消す）/ 字幕: 開始・停止、マイク、言語、翻訳先、モデル、区切り、キー
  - 張り直すときは前の処理が止まりきってから始める。字幕を止めたり張り直したりしたら、確定していない字幕を消す（`discarded`）。字幕は、話す言語の名前が入った辞書ができてから用語のヒントを作る
  - 始められない・止まった理由（キーがない、マイクが開けない、マイクの入力が途切れた、など）は `transcription.status`（トピック `status`）で操作パネルに出す

```ts
interface Settings {
  transcriptionEnabled: boolean;   // 字幕の開始・停止（既定 false）
  micLanguage: LangCode;           // 既定 "ja"
  micDevice?: string;              // なければ OS の既定の入力デバイス
  translateTo: LangCode[];         // 既定 ["en"]。最大 4。micLanguage は除く
  subtitleDisplay: SubtitleDisplay; // 字幕オーバーレイの見せ方（11 章）
  rankLanguage?: LangCode;         // ランク名の言語。なければ話す言語
  models: { realtime; batch; translate; readings; reasoningEffort? };
  realtimeDelay: string;           // 既定 "low"（3.1）
  segmentation: { minSilenceMs: number; maxSegmentMs: number };  // 既定 750 / 8000
  rankAccount?: number;            // Steam の 32bit アカウント ID
  rankPollSeconds: number;         // 既定 180
}
```

操作パネルからは `SettingsPatch`（変える項目だけ）を送る。`rankAccount` は文字列で受け、SteamID64 でも 32bit の ID でも読む（JS の number では SteamID64 を正確に扱えないため）。読めない値は何も変えずに 400 を返す。

## 6. 言語

### 6.1 方針

- 内部の言語コードは BCP 47。deadlock-api の言語名と音声認識の ISO 639-1 コードは対応表で変換する。
- **配信者が話す言語（`mic.language`）も翻訳先も、対応表のどれでも選べる**。日本語の配信者を前提にしているので既定は `ja` だが、言語ごとの分岐はコードに書かない。
- 制約になるのは deadlock-api（公式名がある言語）と LLM の翻訳の質だけ。音声認識の対応言語が効くのは `mic.language` だけ。

### 6.2 対応表（`lang.rs` → ts-rs で TS に出す）

| LangCode | deadlock-api | 音声認識 | 表示名 |
|---|---|---|---|
| `ja` | `japanese` | `ja` | 日本語 |
| `en` | `english` | `en` | English |
| `ru` | `russian` | `ru` | Русский |
| `zh-Hans` | `schinese` | `zh` | 简体中文 |
| `zh-Hant` | `tchinese` | `zh` | 繁體中文 |
| `ko` | `koreana` | `ko` | 한국어 |
| `th` | `thai` | `th` | ไทย |
| `vi` | `vietnamese` | `vi` | Tiếng Việt |
| `id` | `indonesian` | `id` | Bahasa Indonesia |
| `de` | `german` | `de` | Deutsch |
| `fr` | `french` | `fr` | Français |
| `es` | `spanish` | `es` | Español |
| `es-419` | `latam` | `es` | Español (Latinoamérica) |
| `pt-PT` | `portuguese` | `pt` | Português |
| `pt-BR` | `brazilian` | `pt` | Português (Brasil) |
| `it` | `italian` | `it` | Italiano |
| `nl` | `dutch` | `nl` | Nederlands |
| `pl` | `polish` | `pl` | Polski |
| `uk` | `ukrainian` | `uk` | Українська |
| `cs` | `czech` | `cs` | Čeština |
| `tr` | `turkish` | `tr` | Türkçe |
| `el` | `greek` | `el` | Ελληνικά |
| `hu` | `hungarian` | `hu` | Magyar |
| `ro` | `romanian` | `ro` | Română |
| `bg` | `bulgarian` | `bg` | Български |
| `sv` | `swedish` | `sv` | Svenska |
| `da` | `danish` | `da` | Dansk |
| `no` | `norwegian` | `no` | Norsk |
| `fi` | `finnish` | `fi` | Suomi |

29 言語の中に右から左に書く言語はないので、RTL への対応は要らない。

### 6.3 表示

- 描画する要素にはすべて `lang` 属性を付ける。CJK は同じ漢字でも言語によって字形が違うため。
- フォントは Windows のシステムフォントを優先し、同梱はしない。

```css
:lang(ja)      { font-family: "Noto Sans JP", "Yu Gothic UI", "Meiryo", sans-serif; }
:lang(zh-Hans) { font-family: "Noto Sans SC", "Microsoft YaHei UI", sans-serif; }
:lang(zh-Hant) { font-family: "Noto Sans TC", "Microsoft JhengHei UI", sans-serif; }
:lang(ko)      { font-family: "Noto Sans KR", "Malgun Gothic", sans-serif; }
:lang(th)      { font-family: "Noto Sans Thai", "Leelawadee UI", sans-serif; }
```

### 6.4 ふりがな

- ルビは**日本語のテキストにだけ**付ける。配信者が日本語で話すとき（既定）は原文に付き、翻訳先に `ja` を選んだときはその訳文の用語（辞書の読み）に付く。
- 原文の読みは文ごとに LLM（辞書の名前の読みと同じ `models.readings`、推論なし）に聞く（`furigana.rs`、9.2）。文脈で読みが決まる語（一人・上方修正・金・一日中）や造語も正しく読む。形態素解析（Lindera + IPADIC）は造語や文脈に弱く（「上方(かみがた)修正」「金(かね)」）、併用すると実装が増えるだけなので使わない。読みが取れなかった文はふりがななしで出す（ふりがなは補助なので困らない）
- 直前の発話を文脈に付ける。比較（`experiments/furigana_context.py`、文脈で読みが変わりうる短い発話と対照の 10 件 × 5 回。文脈はアプリと同じく 3 発話で、1 つは関係のない雑談）: 文だけでも金(かね)・一日(ついたち)・辛(つら)い・角(かど)・一枚上手(うわて)は正しく、誤ったのは「人気ないね」の人気(にんき)だけだったが、文脈（「このレーン誰もいない」）を付けると 5/5 で ひとけ になった（全体で 45/50 → 50/50）。文脈を付けても区切りが崩れる答えはなかった

## 7. 字幕パイプライン

### 7.1 Segment（1 発話 = 1 Segment）

```ts
type Stage = "partial" | "final" | "enriched" | "discarded";

interface Segment {
  id: string;
  startedAt: number;          // epoch ms。表示順のキー
  stage: Stage;
  lang: LangCode;             // 話された言語（= mic.language）
  text: string;               // 最新の認識テキスト（partial → final で上書き）
  enriched?: Enriched;        // stage === "enriched" のとき
  settledAt?: number;         // 表示時間の起点: enriched になった時刻、または enrich が失敗した時刻
}
```

### 7.2 状態遷移

```
mic ─┬─▶ ローカル VAD（earshot）── 発話の開始 → Segment を開く
     │                         └─ 無音 ≥ minSilenceMs、または長さ ≥ maxSegmentSeconds → Segment を閉じる
     │
     └─▶ GPT-Live-Transcribe（WS。セッションを張りっぱなしにする。Segment を閉じたら commit）
           │ delta（item_id 付き）→ その item の Segment に upsert(stage=partial, text += delta)
           ▼
  Segment が閉じたら、リングバッファから区間 [start-300ms, end+300ms] を切り出す
         ▼
  GPT-Transcribe（バッチ）+ keywords  → upsert(stage=final, text)
         │   失敗したとき: realtime で溜めたテキストを final にする
         ▼
  GPT-6.1 Terra（構造化出力。9 章）
         ├─ kind=speech → upsert(stage=enriched, enriched, settledAt)
         ├─ kind=noise  → upsert(stage=discarded)
         └─ 失敗・タイムアウト → final のまま settledAt を付ける（翻訳なしで表示して消える）
```

- **区切りはローカルの VAD で決める**（3.1 の「発話の区切り方」を参照）。
- **delta と Segment の対応**: 字幕の流れは音声認識に「開いた（Segment の ID）」「音声」「閉じた」を順に送るだけ。捨てる短い発話も閉じる。item は commit した順に並ぶので、初めて見る `item_id` は「commit したが確認がまだの Segment のうち最も古いもの」、なければ「今話している Segment」のものとする（3.1）。
- Segment ごとに独立した非同期タスクで処理する。enrich の完了順が前後しても、表示順は `startedAt` で固定する。
- enrich には直前 2〜3 発話の final テキストを文脈として渡す（代名詞や話の流れのため）。
- Realtime セッションが切れたら、新しいセッションを張り直す。切れている間の音声は捨て、今の Segment の続きから送る（final はバッチで取れる）。

### 7.3 遅延の目安

| 段階 | 目安 |
|---|---|
| partial | 話し始めから約 1 秒（`delay: low`） |
| final | 発話終了から 1〜2 秒 |
| enriched | さらに 2〜4 秒 |

字幕と画面のずれを減らしたいときは、OBS 側で映像に遅延をかける。

## 8. 用語辞書（glossary）

### 8.1 データソース（deadlock-api、確認済み）

| 対象 | エンドポイント | 備考 |
|---|---|---|
| ヒーロー | `GET /v1/assets/heroes?language=…` | `player_selectable && !disabled` で絞る。**`only_active` / `in_development` では絞らない**（最近リリースされたバイオレットは `in_development: true` のまま）。`images.icon_image_small(_webp)` |
| アイテム | `GET /v1/assets/items?language=…` | `type=upgrade && shopable` が 173 個。すべてに `shop_image(_webp)` がある |
| スキル | 同じエンドポイントの `type=ability` | ヒーローの `items`（signature スロット）から、実際に使われるものだけを抜き出す |
| ランク | `GET /v1/assets/ranks?language=…` | 名前・色・バッジ画像 |
| バージョン | `GET /v1/assets/client-versions` | 更新の検知 |

- `id` は全言語で共通なので、言語の対応づけに使う。
- レート制限は 1 IP あたり 200 req/分。CORS は `*`。

### 8.2 生成と更新（`crates/core/src/assets/`）

アプリの起動時に `client-versions` の最新を確認し、その版のキャッシュがなければ作る。

1. **`mic.language`・`en`・`translateTo` の言語だけ**を取得し、`id` で結合する（アイテム一覧は 1 言語で約 7MB あるので、29 言語をまとめて取らない）。どのエンティティを載せるかは英語版で決める（英語名がないものは載せない）。
2. アイコンを `{データディレクトリ}/assets/{形式}/{clientVersion}/icons/{kind}/{id}.webp` にダウンロードし、`/assets/{clientVersion}/icons/...` で配信する（配信中に外部へ依存しない。URL に版を含むので `immutable` で長くキャッシュできる）。ファイルは一時ファイルに書いてから置き換える。取れなかったアイコンはそのエンティティから外して表示せず、辞書も保存しない（次の起動で作り直し、欠けた分だけ取り直す）。
3. `data/aliases.ja.json` と `data/readings.ja.json`（キーはエンティティの参照）をマージする。読みは、つなげた文字列が今の日本語名と一致するときだけ使う（パッチで改名されたら古い読みは捨てる）。
4. **辞書の漢字名の読みは LLM（`models.readings`、既定 `gpt-6.1-terra`、推論なし）に読ませ、名前ごとにキャッシュする**（`assets/readings.rs`）
   - 辞書は読みなしで保存し、読み込むたびに「手書きの読み → キャッシュ（`{データディレクトリ}/assets/readings-llm.ja.json`）の LLM の読み」の優先順で重ねて配信する。キャッシュにない名前は配信した後に 40 件ずつ問い合わせ、届いたら配信し直す
   - 辞書の保存と切り離しているので、キーが無効・未設定でも辞書は保存され（読みなしで動く）、キーを後から設定すれば次の起動で効く。問い合わせが失敗したら次の起動で問い直す
   - 答えは、つなげると名前になること・読みがひらがなであること（カタカナはひらがなにする）・漢字には必ず読みがあることを確かめ、読みを漢字の部分ごとに割り当てる（かなの部分を読みの中で探して区切る。「取り扱い」→ 取(と)り扱(あつか)い、「魔呪の罠」→ 魔呪(まじゅ)の罠(わな)。字幕の文のふりがなにも同じ確かめ方を使う）。通らなかった名前はキャッシュせず、次の起動で問い直す（答えは揺れるので、たいてい次は通る）
   - キャッシュが壊れていたら `.broken` に退けて空から始める。読めない（権限など）ときは LLM の読みを使わない（黙って上書きしない）
   - client 6839: 漢字を含む名前 35 件すべてに正しい読みが付いた（人狼・屍鬼累々・魔瓶・聖別手榴弾・五芒星・魔呪・一意専心・餌付け・袋叩き など）。初回の問い合わせは約 19 秒、2 回目以降はキャッシュから
   - 比較（35 件、判断が難しい 10 件で採点）: GPT-6.1 Terra（推論なし）と Claude Sonnet 5.5（推論あり low）が 0 件、Sonnet 5.5（推論なし）1 件、Mistral Large 4 2 件（一意→いつい、餌付け→えさつけ）、Haiku 5.5 2〜3 件。形態素解析（IPADIC）は造語 7 件を誤る（知らない複合語を 1 文字ずつ訓読みする。「人(ひと)狼(おおかみ)」）
   - `data/readings.ja.json` は、それでも直したい名前を手で上書きするためだけに残す（今は空）。辞書には保存せず読み込むたびに重ねるので、書き足せばすぐ効く。名前が変わって合わなくなった手書きの読みは使わず、LLM の読みにする
   - 推論の深さは `models.reasoning_effort`（既定 `none`。推論しないモデルにするときは未設定にしてパラメータを送らない）

- キャッシュに必要な言語が揃っていれば作り直さない（2 回目以降の起動は 1 秒未満）。初回の生成は 3 言語で約 8 秒。`languages` には実際に名前が取れた言語だけを載せる。
- `{形式}`（`CACHE_FORMAT`）は Glossary の形やアイコンの置き方を変えたら上げ、古いキャッシュを読まないようにする。
- 同じスキルを複数のヒーローが持つ場合は、最初のヒーローのものとして 1 件だけ載せる。
- 通信できないときは、手元にある最新の版を使う（言語が足りなくても使う）。
- データディレクトリは Tauri の `app_data_dir`（ヘッドレスでは `dirs::data_dir()/dev.deadlock-stream-overlays.app`。同じ場所）。
- 辞書ができたら `glossary.updated` を流し、オーバーレイは `GET /api/glossary` を取り直す（まだないときは 503）。`glossary.updated` は接続直後と取りこぼし後にも送る（10 章）。
- 実測（client 6839）: ヒーロー 44、アイテム 173、スキル 176。

### 8.3 形式（`types.rs` の `Glossary`）

```jsonc
{
  "clientVersion": 6839,
  "languages": ["en", "ja", "ru"],
  "entities": {
    "hero:13": {
      "kind": "hero",
      "className": "hero_haze",
      "names": { "en": "Haze", "ja": "ヘイズ", "ru": "Пелена" },
      "aliases": {},
      "icon": "/assets/6839/icons/hero/13.webp"
    },
    "item:1548066885": {
      "kind": "item",
      "className": "upgrade_clip_size",
      "names": { "en": "Extended Magazine", "ja": "基礎拡張マガジン", "ru": "Увеличенный магазин" },
      "ruby": [{ "text": "基礎拡張", "ruby": "きそかくちょう" }, { "text": "マガジン" }],
      "aliases": { "ja": ["拡張マガジン"] },
      "icon": "/assets/6839/icons/item/1548066885.webp",
      "slot": "weapon"
    },
    "ability:731943444": {
      "kind": "ability",
      "className": "ability_bullet_flurry",
      "names": { "en": "Bullet Dance", "ja": "バレットダンス", "ru": "Танец пуль" },
      "aliases": {},
      "icon": "/assets/6839/icons/ability/731943444.webp",
      "hero": "hero:13"
    }
  }
}
```

- エンティティの参照は `"{kind}:{id}"`（`hero:13`, `item:…`, `ability:…`）で統一する。
- `ruby` は日本語の名前の読み（`names.ja` 用）。漢字を含む名前だけに付ける。
- `slot`（`weapon` / `vitality` / `spirit`）はアイテムの色分けに使う。
- 描画では、日本語の段落でも発話された略称ではなく公式名を出す（`拡張マガジン` → `基礎拡張マガジン`）。辞書にない言語の名前は英語名で出す。

### 8.4 音声認識への用語ヒント

`keywords` には、`mic.language` での名前と別名を入れる（上限 500 件。送りすぎない）。すべての公式名（ヒーロー → アイテム → スキル）を先に入れ、余った枠に別名を入れる。別名は音声認識より、話した言葉を用語に結びつける照合（9 章）の側で効く。

### 8.5 別名（`data/aliases.ja.json`）

日本のプレイヤーが口で言う呼び方を、公式名と同じ用語に結びつける（例: 拡マガ / カクマガ / エクステンデッドマガジン → 基礎拡張マガジン、カース → 呪いの遺物）。アプリに同梱し、辞書を読み込むときに `api.deadlock.jp` からも取る（17 章）。同梱の別名ですぐ配り、サーバーの別名が届いたら足して配り直す（起動を待たせない。サーバーの分が古くても空でも、同梱の別名は消えない）。辞書のキャッシュには保存せず、読みと同じく配るたびに重ねる（アプリの更新で別名が変わったら、Deadlock の更新を待たずに効く）。

- 中身: コミュニティの略称、FPS 一般の略し方（拡マガ）、昔の公式訳（基本マガジン、治療弾）、英語名のカタカナ読み（インフェルナス）。漢字かな混じりの別名は、音声認識が書くカタカナだけの形も入れる（拡マガ と カクマガ）
- 作り方（開発のとき。Deadlock の更新で用語が増えたら、その分だけ）:
  1. 通称を Web で調べ、出典の URL 付きの候補を `experiments/results/aliases/*.output.json` に書き出す（Claude Code のセッションで並行して調べた。API で回すなら `experiments/aliases_research.py`）。Web に書かれた例は昔の訳や表記ゆれが多く、口で言うだけの略称はほとんど見つからない。FPS 一般の略し方を含めるよう指示すると拡マガが見つかる
  2. 英語名のカタカナ読みを GPT-6.1 Terra で作る（`experiments/katakana_names.py`）。使用例がなくても、英語名で言えば音声認識はカタカナで書くので入れる
  3. `experiments/merge_aliases.py` でまとめる。英語のままの旧名、誤記、5 文字未満のカタカナ読み（エヴァ、コンボのようにふつうの言葉と重なる）、どれかの公式名そのものは入れない
- 照合では公式名を常に優先する。2 つの用語に同じ別名が付いたら、どちらとも決められないので使わない

### 8.6 スキルの番号（「ヘイズのウルト」）

辞書のスキルに、持ち主の何番目のスキルか（`abilitySlot`。API の `signature1`〜`signature4`。4 がウルト）を入れる。照合でヒーロー名のすぐ後ろが次の形なら、そのヒーローのそのスキルにする。

- 日本語: 「（の）ウルト / アルティメット / ウルティメット」（「ヘイズウルト」のように「の」がなくてもよい）、「（の）1スキ / 1スキル / 1番スキル / 1番目のスキル / 一番スキル」（数字は半角・全角・漢数字）。「1番強い」「一番好き」と区別できないので、番号は「スキル」「スキ」まで言ったときだけ。「ウルトラ」「1スキン」のように長いカタカナ語の一部なら違う
- 英語: 「Haze's ult」「Haze ultimate」
- 原文では話した言葉のまま（ウルト）にスキルのアイコンと色を付ける。翻訳では公式名になる（Haze's [アイコン] Bullet Dance）。別名も同じ（11 章）

## 9. enrich（`enrich.rs`, `furigana.rs`）

### 9.1 方針: LLM に任せるのは翻訳と読みだけ

最初は「トークン分け・ふりがな・固有名詞・翻訳・雑音の判定」をすべて LLM に返させたが、実測で次の問題が出たので役割を分けた（16 章）。

| 問題（Mistral Small 4） | 対応 |
|---|---|
| ふりがながずれる（`僕(ぼ)`、`最(さい)近(き)`）、送り仮名ごと付ける（`強い(つよい)`） | 読みは**語ごとの区切りと読み**だけを返させ、つなげると原文になること・読みがひらがなであることを確かめ、送り仮名を外す割り当てはコードでする（GPT-6.1 Terra で実測。形態素解析より造語・文脈に強い）。初めは形態素解析（Lindera + IPADIC）にしたが、文脈で決まる読み（上方修正・金・一人）を誤るのでやめた |
| 名前の途中で区切る（`ヘ⟨イズ⟩`）、同じ名前を付けたり付けなかったりする | **辞書の名前と略称の最長一致**で固有名詞を決める |
| 普通の文を「雑音」と判定して消す | 判定しない。咳やフィラーだけの音声はバッチの文字起こしが空になり、そこで捨てる |
| 訳文で `{id}` を使わず名前をそのまま書く（`Haze's`） | LLM に名前を見せない。固有名詞は中身のない記号 `⟦1⟧` にして種類（hero など）だけ伝え、訳文の記号をエンティティに戻す |

### 9.2 処理の流れ

1. final のテキストを、辞書の名前と略称（話す言語、2 文字以上）で最長一致に分ける。カタカナの名前は、前後もカタカナなら別の語の一部として一致させない（「セブンイレブン」の「セブン」）
2. 日本語で漢字を含む文なら（ふりがなを出す設定のときだけ）、文全体の読みを LLM（`models.readings`）に聞く（3 と同時に。1 文 2〜3 秒。翻訳先がなくても聞くので、そのときは enriched がその分遅れる）。直前 3 発話を文脈に付ける。答えは 8.2 と同じく確かめて漢字の部分ごとに割り当て、文字の位置で 1 の区切りに当てはめる。用語の中に収まる読みは用語のルビ（`Token` の `ruby`。「一番(いちばん)スキル」）、区切りをまたぐ読みは付けない。答えが通らなければふりがななし
3. 固有名詞を出てきた順に `⟦1⟧`, `⟦2⟧` …（同じものは同じ番号）にした原文と、各記号の種類、直前 3 発話を LLM に渡し、記号入りの訳文を返させる
4. 訳文の `⟦n⟧` をエンティティに戻す。知らない番号は捨て、空いた空白を詰める

- 辞書は enrich のたびに store から取り、版が変わったら照合器を作り直す（辞書が後から届いても固有名詞が付く）
- 照合は、空白で区切る言語では単語の境界を見て、大文字小文字を区別しない（「Hazel」の「Haze」は一致させない）
- 翻訳先がないときは翻訳の LLM を呼ばない（ふりがなは聞く）

翻訳に失敗しても、1・2 の結果（固有名詞とふりがな）は付けて enriched にする。翻訳先がなく漢字もない文は、LLM を呼ばない。

### 9.3 LLM の入出力

- モデル: OpenAI `gpt-6.1-terra`（設定の `models.translate`、`reasoning_effort: none`）。速さより訳の質を優先した（確定字幕から翻訳まで 1.7〜2.6 秒）
  - Mistral Large 4 は推論を止められず（`reasoning_effort` は low / medium / high のみ）、1 発話に 21〜24 秒かかり、出力が途中で切れることもあったので使わない
  - Z.ai GLM 5.3 Flash（`zai-glm-5-3-flash`）も推論するモデルで、1 発話 5.5〜23 秒。訳の質は Small 4 とほぼ同じ。ふりがなを付けさせると普通の文は正しいが 20 秒かかり、造語の多い文では原文と無関係な文を作った
  - Small 4 は 1 発話 0.8〜1.8 秒、`⟦n⟧` は 4/4 で保たれた
  - OpenAI `gpt-6-luna`（`reasoning_effort: none`）は 1.1〜3.4 秒。ロシア語訳に日本語の「実況」が残った
  - OpenAI `gpt-6.1-terra`（同）は 1.7〜3.8 秒。所有格のルールを守り、ロシア語がいちばん自然 → **採用**
  - Claude Haiku 5.5（推論なし）2.4〜3.9 秒、Claude Sonnet 5.5（推論なし）2.3〜9.2 秒。Sonnet の訳は自然
- system: 役割とルール（名前の一覧は渡さない）
- user: `{ "context": [直前の確定字幕], "transcript": "⟦1⟧のウルト強い", "terms": { "⟦1⟧": "hero" } }`
- OpenAI の API キーがなければ翻訳しない（固有名詞とふりがなは付ける）
- 構造化出力（`response_format: json_schema`, `strict`）: `{ "translations": { "en": "...", "ru": "..." } }`。`translations` のキーは設定の翻訳先から毎回作る
- タイムアウト 8 秒

### 9.4 出力の型

```ts
type Token =
  | { t: "text"; text: string; ruby?: string }      // ruby は日本語の漢字を含む語にだけ
  | { t: "entity"; ref: string; spoken?: string; ruby?: RubySegment[] };
    // spoken = 原文で発話された表現（そのまま出す）。ruby = その読み（漢字を含むときだけ）

interface Enriched {
  source: Token[];                               // 原文（固有名詞 + ふりがな）
  translations: Partial<Record<LangCode, Token[]>>;
}
```

### 9.5 制約

- 訳文の固有名詞は辞書の形（主格）のまま差し込むので、格変化する言語（ロシア語など）では文法的に不自然になることがある（`У Пелена`）
- 日本語への翻訳（話す言語が日本語以外のとき）は、訳文にふりがなが付かない（訳文は文字列で返させているため）。必要になったら訳文にも 1・2 を通す

## 10. イベントプロトコル（WS `/ws`）

差分ではなく、**最新の状態をそのまま upsert** する。

```ts
// client → server
type ClientMsg = { type: "subscribe"; topics: ("subtitle" | "glossary" | "rank" | "status" | "display" | "level" | "usage")[] };

// server → client
type ServerMsg =
  | { type: "snapshot"; segments: Segment[]; rank?: RankState }  // 接続直後
  | { type: "segment.upsert"; segment: Segment }
  | { type: "rank.update"; rank: RankState | null }               // null: アカウントを外した・替えた
  | { type: "transcription.status"; error: string | null }        // 操作パネル向け。字幕を始められない・止まった理由
  | { type: "glossary.updated"; clientVersion: number }           // オーバーレイが辞書を取り直す
  | { type: "display"; subtitles: SubtitleDisplay; rankLanguage: LangCode; itemCards: { holdSeconds: number; language: string } }  // オーバーレイの見せ方（11・12 章）
  | { type: "audio.level"; db: number }                           // 操作パネルのメーター。字幕を動かしている間、0.1 秒ごとのピーク（dBFS）
  | { type: "usage"; usage: UsageView };                          // 操作パネルの料金。使うたびに
```

- 辞書は `GET /api/glossary` で取得し、オーバーレイ側で持つ（Segment には ID しか載らない）。
- サーバーは直近の Segment を（例: 50 件）保持して、snapshot で送る。
- WS クライアントは指数バックオフで再接続し、再接続したら snapshot で状態を置き換える。
- 接続直後と broadcast の取りこぼし（Lagged）の後は、購読しているすべてのトピックの現在の状態（`snapshot`、辞書があれば `glossary.updated`、ランクがあれば `rank.update`、`transcription.status`、`display`、`usage`）。`audio.level` は持たずに流すだけを送る。

## 11. 字幕オーバーレイ

URL: `/overlay/subtitles`（**パラメータはない**。OBS に一度貼れば、あとは貼り直さない）

オーバーレイは**字幕の箱**（推奨 幅 1000・高さ 400）。置く場所は OBS で決める（Deadlock は画面下中央にスキル欄があるので、その上や画面の左右に置く）。字幕は箱の下から積み上げ、箱の幅で折り返す。大きさを変えるときはソースを引き伸ばさず、プロパティの幅と高さを変える（引き伸ばすと文字ごと伸びる）。

見せ方は操作パネルで変え、設定（`subtitleDisplay`）に保存する。変更は変えた項目だけを送り、サーバーが今の値に重ねる（続けて変えたとき、前の変更を古い値で戻さない）。大きさと色は操作パネルの値が優先する（OBS のカスタム CSS で変えるときは `.captions` に `!important` で書く）。大きな文字・多い行数・多い翻訳先を組み合わせたら、箱の高さも上げる（はみ出した上の字幕は切れる）。設定ファイルに読めない項目があれば、その項目だけ既定値にする（ほかの設定は残す）。縁取りの太さは行ごとの文字の大きさに合わせる。変えたら `display`（トピック `display`）で表示中のオーバーレイに届き、その場で切り替わる。URL を手で書き換える運用はしない。

| 項目 | 意味 | 既定値 |
|---|---|---|
| `furigana` | ふりがなを出すか。出さないときは読みを問い合わせない（切り替える前の字幕には付かない） | `true` |
| `align` | 揃え（`left` / `center` / `right`）。箱を置く位置に合わせる | `center` |
| `fontSize` | 話した言葉の文字の大きさ（px。16〜96） | `40` |
| `translationFontSize` | 翻訳の文字の大きさ（px。16〜96） | `26` |
| `textColor` | 確定した字幕の文字色（`#rrggbb`） | `#ffffff` |
| `heroColor` | ヒーロー名の色（ゲームに決まった色がないので選べる） | `#ffd34d` |
| `abilityColor` | スキル名の色 | `#8fd8ff` |
| `background` | 文字の後ろ（`outline` = 縁取りだけ / `band` = 行ごとに半透明の黒い帯。明るい場面でも読める） | `outline` |
| `maxLines` | 同時に出す字幕の数（1〜5） | `3` |
| `holdSeconds` | 確定してから消すまでの秒数（2〜30） | `6` |

話した言葉の字幕は常に出す（翻訳だけを出す設定は作らない。翻訳ができるまで何も出ず、リアルタイムに伸びる字幕の良さがなくなるため。話した言葉だけにしたいときは翻訳先を 0 にする）。訳文は翻訳先（`translateTo`）をすべて出す。出す言語を減らしたいときは翻訳先を減らす（そのぶん費用も減る）。

### 表示ルール

- 映画字幕のような**中央揃え**。下から積み上げ、最新の発話を一番下に出す。partial の行は下端でその場で更新される。
- 表示時間は `settledAt` から数え、`holdSeconds + 文字数 × 0.15 秒` が過ぎたらフェードアウトする。partial / final の間は消さない。
- `maxLines` を超えたら、時間に関係なく古いものから上に押し出す（`animate:slide`。`lib/slide.ts`）。
  - `animate:flip` は使わない。flip は大きさの変化も scale で補間するので、partial → final で行の幅が変わると文字が潰れて見える。
- 文字の変化:
  - partial が伸びる・final で句読点や誤認識が直るときは **torph**（`torph/svelte` の `TextMorph`）で文字単位に動かす。増えた文字は入り、消えた文字は抜け、残る文字は位置だけ動く
  - enriched になったら文字はそのままで、ふりがな・アイコンをフェードインし、用語の色を白から変える（CSS のアニメーション）。torph はプレーンなテキストしか扱えないので、ルビやアイコンの入る enriched には使わない。訳文は下からふわっと出す
  - 確定前の灰色から白へは色だけ切り替える
- `discarded` になったらすぐに消す。

### 1 発話の中身

```
   [アイコン]インファーナスのultに気(き)をつけて      ← 原文（大きく。ふりがなは <ruby> で上に出す）
   Watch out for [アイコン]Infernus's ult             ← 訳文（小さく。翻訳先の分だけ並べる）
```

- エンティティはアイコンを付けてハイライトする（色は種類ごと: hero / item / ability）。原文では話した言葉のまま（拡マガ、ウルト、一番スキル）、訳文では公式名を出す。話した言葉を公式名に置き換えると、「ヘイズの一番スキルって何だっけ」が「ヘイズのスリープダガーって何だっけ」になり、字幕として話と食い違う
- 原文の用語のふりがなは、公式名どおりに話していれば辞書の読み（名前ごとに確かめたもの。8.2）、別名なら文の読み（9.2）
- partial は灰色、final は通常の色（翻訳待ち）、enriched で完成形になる。

## 12. ランクオーバーレイ（`rank.rs`, `/overlay/rank`）

URL: `/overlay/rank`（パラメータはない。ブラウザソースは幅 480・高さ 140）。ランク名の言語は操作パネルで選ぶ（`rankLanguage`。話す言語・英語・翻訳先から。既定は話す言語）。名前がない言語は英語で出す

表示は **アイコン（サブランク付き）・ランク名（サブランク付き。例: エミッサリー III）・今のサブランクの中での進み具合のゲージ（例: 650 / 1,000）**。出すものは操作パネルで選ぶ（`rankContent`: アイコン・名前・ポイント / アイコンと名前 / アイコンだけ）。サブランクはゲームと同じローマ数字（I〜VI）か、算用数字（1〜6）から選ぶ（`rankNumerals`）。

- 色は字幕と同じく白に黒の縁取り（ゲージの塗りも白）。deadlock-api にはティアの色があるが、ゲームの中では見かけないので使わない
- エターナス（最上位）は上のティアがないのでゲージを出さない。リーダーボードの地域（`rankRegion`）を選んでいて、載っていれば順位（日本語は「アジア 123位」、ほかは「#123 Asia」）、なければ累積のランクポイント（例: 72,000 RP）を出す
- 順位は `GET /v1/leaderboard/{region}`（Europe / Asia / NAmerica / SAmerica / Oceania。Valve の更新は 1 時間ごと）。各行のアカウントは表示名から推定した候補（`possible_account_ids`）なので、候補が自分の ID だけの行を使う（候補が何人もいる行は同じ名前の別人かもしれないので使わず、ポイントを出す）。リーダーボードは取れてから 15 分経ったら取り直す（取れなければ次の回にまた取りに行き、それまでは前の順位のまま）。2 時間より古い順位は出さない。順位はランク名と同じ言語で書く（ランク名が英語にフォールバックしたら順位も英語の形）

| 用途 | エンドポイント |
|---|---|
| 現在のランク | `GET /v1/players/{account_id}/rank`（最後のランクマッチが終わった時点。`badge` = `tier * 10 + subrank`） |
| ランクポイント | 同じ応答の `last_match.player_rank_final_flat_progress`（累積。サブランク 1 つが 1000、6 番目だけ 2000 なので 1 ティアは 7000。ティア t・サブランク s の始まりは `(t-1)*7000 + (s-1)*1000`。7-3 の始まりが 44,000） |
| ランク名 | `GET /v1/assets/ranks?language=…`（設定の言語の分だけ取る） |
| アイコン | `GET /v1/assets/ranks/{tier}/{subrank}/image?format=webp`（サブランクのないティアは `large_webp`） |

- アカウントは Steam の 32bit アカウント ID（SteamID64 からは `76561197960265728` を引く。どちらでも受け付ける）。操作パネルで設定する
- `rank_poll_seconds`（既定 180 秒）ごとに取りに行き、変わったら `rank.update`（トピック `rank`）を流す。接続直後にも送る
- データの取り込みに時間がかかるので、試合が終わってすぐには反映されない（deadlock-api の bot とフレンドになるか、Patreon の優先取り込みで速くなる）
- アイコン（約 75KB）はディスクに保存せず**バッジごとに**メモリに持ち、`/assets/rank/{badge}.webp` で配信する。取れていない間は `image` を出さず（画像を表示しない）、次の回に取り直す。別のバッジのアイコンを配信しない
- ランク名は言語ごとに取り、失敗した言語だけ次の回に取り直す。外部 API の呼び出しには既定のタイムアウト（接続 10 秒、全体 60 秒）を付け、応答が返らずに止まらないようにする
- アカウント ID を読み取れなければ警告してランクを止める
- 並びは 1 行目にランク名、2 行目に太いゲージ。ポイント（`650 / 1,000`。分母は小さく）はゲージの中央に縁取りした白い文字で重ねる（白い塗りの上でも暗い地の上でも読める）。ポイントが変わったら torph で桁ごとに数字を動かす
- バッジが変わると拡大しながら入れ替わり、ゲージは伸び縮みする
- 当日の勝敗などは今は出さない（必要になったら `match-history` を使う）

### 12.5 アイテム説明オーバーレイ（`/overlay/items`）

字幕でアイテム名が出たら、そのアイテムの説明をゲーム内のツールチップと同じカードで出す。ブラウザソースは幅 460・高さ 560（カードは幅 450 で、箱の左上から出る）。

- カードは [deadlock-ui](https://github.com/deadlock-api/deadlock-ui)（`@deadlock-api/ui-core`。Stencil 製の Web Components、MIT）の `dl-item-tooltip` を使う。Web Components なので Svelte からそのまま使える。使う部品だけを `dist/components/*.js` から読み込む。開発中のライブラリなのでバージョンは固定する
- 説明のデータは部品が deadlock-api から取る（こちらでは持たない）。言語は `<dl-provider language>` で渡す
- 翻訳まで済んだ字幕（`enriched`）の用語から `kind = item` のものを話した順に拾い、辞書の `className` でカードを出す。1 枚ずつ `holdSeconds`（既定 8 秒）出し、続けて話したものは順番に出す
- 出しているアイテムをもう一度話したら、残りの時間を最初に戻す。待っているアイテムなら何もしない。待ちは 3 枚までにし、それより古いものは捨てる（話から大きく遅れない）
- 接続した時点で説明まで済んでいた字幕では出さない（つなぎ直すたびに出し直さない）。途中の字幕は説明が付いたときに拾う。辞書を読む前に届いた字幕は、読めてから拾う
- 設定（`itemCards`）: 出しておく時間（3〜30 秒。読み込んだ値が範囲外なら既定値）、説明の言語（既定は話す言語）。`display` メッセージで deadlock-api の言語名（例: `japanese`）にして届ける。deadlock-ui が対応していない言語は英語で出す

## 13. 操作パネル（control.html）

Tauri のウィンドウでも OBS のカスタムブラウザドックでも同じ画面（`http://127.0.0.1:4510/`）。shadcn-svelte（Tailwind CSS v4）で作る。オーバーレイは軽さと OBS のカスタム CSS での上書きやすさを優先し、Tailwind を入れずに素の CSS のまま。

- **上部に固定**: 状態（停止中 / 準備しています / 字幕を出しています / 音が届いていません / 止まっています）と、開始・停止のボタン
- **注意はタブの上に出す**: OpenAI のキーがない、字幕が止まった理由、設定を保存できない
- **URL を手で書き換えさせない**: 見せ方はすべて画面の操作で変え、OBS に貼る URL はコピーするだけ

| タブ | 内容 |
|---|---|
| マイクと翻訳 | 入力デバイス（一覧の更新つき。見つからなければそう出す）、入力レベルのメーター（-60〜0 dB）、5 秒間 -60 dB を超えなければ「音が届いていません」、話す言語、翻訳先（最大 4。OpenAI のキーがなければ注意）、文の区切り（すぐ 0.5 秒 / ふつう 0.75 秒 / つなぐ 1 秒。ミリ秒は「数値で指定」のダイアログ） |
| 字幕の表示 | プレビュー（オーバーレイを推奨サイズ 1000×400 で開いて縮めたもの）とテスト字幕、11 章の見せ方（揃え、文字の大きさは 小 / 中 / 大 / 特大 の組み合わせ、背景は縁取り / 半透明の帯。大きさを数値で決めるときと、色（文字・ヒーロー名・スキル名。どれも明るい色の候補、OS の色選び、#rrggbb）は「詳しく設定」のダイアログ。アイテム名はゲーム内のスロットの色で固定）。並べた選択肢で足りない値は、どの設定も見出しの横のボタンから開くダイアログで決める（変えた項目だけを送る）、OBS に貼る URL と置き方 |
| アイテム説明 | プレビュー（460×560）とテスト字幕、出しておく時間（5 / 8 / 12 / 20 秒か「数値で指定」）、説明の言語、OBS に貼る URL |
| ランク | プレビュー（480×140）、Steam ID（SteamID64 かフレンドコード。空で非表示）、ランク名の言語、OBS に貼る URL |
| API キー | OpenAI（文字起こし・翻訳・読み）。状態（保存済み・未設定・読めない。読めないときは理由を出す）、キーの作成ページへのリンク、新しいキーの保存・削除。キーそのものは表示しない |
| 料金 | OpenAI の料金の概算を、使い道（途中の字幕・確定した字幕・翻訳・ふりがな・辞書の読み）ごとに「起動してから」と「今月（UTC）」で出す。正確な額は OpenAI の使用量のページへ |

**料金の概算**（`usage.rs`）: 音声は送った長さ（途中の字幕は commit しなかった分も含む。確定の字幕は WAV の長さ）、LLM は応答の `usage` のトークン数（キャッシュから読んだ分は安い単価）を数え、モデルの単価を掛ける。単価はアプリに持つ（OpenAI の料金ページ、2026-11）。知らないモデルを設定したら、その分は合計に入れず「単価が分からない」と出す。途中の字幕は 0.1 秒ごとに音声を送るたびに数える（接続が切れても送った分は数える）。応答が届かなかった分（タイムアウトなど）は数えられないので、少なめに出ることがある。画面には最新の値だけを流し（`watch`。入力レベルと同じく字幕などの broadcast には混ぜない）、月ごとの分は変わってから 5 秒待ってまとめて `{データディレクトリ}/usage.json` に保存する（一時ファイルから置き換える）

- プレビューの iframe は `color-scheme` を中身（ライト）に合わせる。ダークの画面に違う color-scheme の iframe を置くと、背景が白く塗られて透過しない
- Tauri のウィンドウで外部へのリンクを開いたら、ふだんのブラウザで開く（`on_new_window`）

API（すべて `/api`）:

| メソッド | パス | 内容 |
|---|---|---|
| GET | `/settings` | `SettingsView`（設定と API キーの状態） |
| PATCH | `/settings` | `SettingsPatch` を当てて保存し、処理を張り直す |
| PUT | `/keys/{mistral\|openai}` | `{ key }` を資格情報ストアに保存（空文字列で削除）し、処理を張り直す |
| GET | `/devices` | 入力デバイスの名前 |
| GET | `/languages` | 対応している言語のコード。表示名は操作パネルが `Intl.DisplayNames` で画面の言語（日本語）で作る（翻訳先は配信者が読める名前で選ぶ。`pt-PT` は「ポルトガル語」と出す） |

- `/api` は Host ヘッダーが `127.0.0.1` / `localhost` / `[::1]` のときだけ受け付ける（DNS リバインディング対策）。書き換えは JSON だけなので、別のサイトからの単純なフォーム送信も通らない（CSRF 対策）
- 残り: 辞書の状態、直近の Segment のログ、ミュート（グローバルショートカット）

## 14. 実装フェーズ

| Phase | 内容 | 完了条件 |
|---|---|---|
| 0 | 土台: workspace、Tauri + axum + Svelte、WS upsert、ダミーの Segment ✅ | OBS にダミー字幕が流れて消える |
| 1 | 言語の対応表、assets（glossary 生成、アイコンのキャッシュ）、Rich の描画 ✅ | ダミーの Enriched がアイコン・ルビ付きで、複数の言語で表示される |
| 2 | mic → VAD で区切り → Voxtral Realtime（partial）→ Small Transcribe 3（final） ✅（2026-11 に OpenAI へ切り替え。3 章） | 自分の声が字幕になる |
| 3 | enrich（ふりがな、翻訳、エンティティ） ✅ | 3 段階の表示が通しで動く |
| 4 | ランクオーバーレイ ✅ | ランクバッジが表示され、更新される |
| 5 | 仕上げ: 設定の保存と操作パネル ✅、インストーラー、自動更新、見た目（torph など） | 配布できる |

Phase 2 の前に、録音した実況音声で Voxtral 3 の精度（`context_bias` あり / なし、realtime / batch）を確かめる。

## 15. 将来の拡張案（今はスコープ外）

- **ゲーム内 VC・Discord の字幕と逆翻訳**: Windows のプロセス単位の録音（WASAPI Process Loopback）で取れる。Realtime の話者分離（`diarize`, `speaker_references`）が使える。外した理由は、チームメイトの同意、キャラのボイスや効果音の混入による精度の限界、コストと複雑さが倍になること。
- **チャット形式のバブル表示**: 上の機能を入れるときに、左右の配置と一緒に入れる。
- **話す言語の自動判定**: 音声認識は言語を省略すると自動で判定する。ただし短い発話や英単語の多い日本語では誤判定しやすく、用語ヒントの言語も決められなくなるので、今は明示的に設定する。
- **日本語以外の読み補助**（中国語のピンインなど）: Rich の `ruby` の仕組みをそのまま使える。
- **課金と OpenAI の中継**: サーバー（17 章）が OpenAI を代わりに呼び、利用者に API キーを用意させない。利用者の認証と使用量の上限が要る

## 16. 未確認事項（実装前に確かめる）

- [x] Voxtral 3 Realtime の WS プロトコル（3.1 にまとめた）
- [x] 用語ヒント（`context_bias`）
- [x] モデル ID（`/v1/models` で確認）: `voxtral-mini-transcribe-realtime-3`, `voxtral-small-transcribe-3`, `mistral-large-4`
- [x] 日本語の精度（`experiments/stt_eval.py`, sample01: iPad で録った 22 秒の独り言）
  - 文字誤り率は全条件で約 5%。誤りは「えーね」「いや」などのフィラーが落ちたことと「ではある → である」だけ
  - ヘイズ・バイオレット・バレットダンス・ウルト・デッドロックは、`context_bias` なしでも全条件で正解
  - realtime は句読点なし、batch は句読点あり。batch の応答は 22 秒の音声で約 2.7 秒
  - realtime の最初の delta は、発話開始から約 1.5 秒後
  - ターン検出は 22 秒全体で 1 ターンになった → VAD で区切る設計に変えた（3.1）
- [x] パイプラインを通しで確認（`--input-file` で sample01 を実時間で流す）
  - VAD で 4 発話に区切られ、partial → final が順に届いた。final は発話終了から 1〜3 秒
  - 0.43〜0.45 秒の間（`min_silence_ms` 400 より少し長い程度）は区切られず、3 文が 1 つの字幕になった。実際のマイクで長すぎると感じたら 350ms 程度に下げる
  - `context_bias` は 392 語（日本語の名前と略称）で、Realtime もバッチも 400 エラーにならなかった
- [ ] 実際のマイクでの確認（このパイプラインはファイル入力でしか通していない。Windows の WASAPI と macOS のマイク許可を含めて確認する）
- [ ] 音声認識の候補: Microsoft MAI-Transcribe-2（Artificial Analysis の英語中心の WER で 2.0%、Voxtral Small 3 は 2.1%。日本語対応、`phraseList` で用語ヒント、リアルタイム版は public preview で SLA なし）。準備の手間（Azure のリソース）と preview であることから今は採用しない。確定字幕の文字起こしだけなら `stt.rs` で差し替えられるので、正式版になって日本語で上回れば検討する
- [ ] 本番に近い条件（配信用マイク、プレイしながら 5〜10 分、用語を多めに）でもう一度測る。`context_bias` の効果はこの条件でないと見えない
- [ ] Realtime セッションの最大接続時間と、切れたときの挙動
- [x] Large 4 / Small 4 の比較（同じ文で Large 4 は 21〜24 秒・推論 2,400〜2,900 トークン、Small 4 は 3 秒・推論なし）→ Small 4 を採用
- [x] `response_format: { type: "json_schema", json_schema: { name, strict, schema } }` が使える
- [x] enrich を通しで確認（sample01）: 4 発話すべてが enriched になり、ふりがな・固有名詞は正しく、final から 0.5〜2 秒
- [ ] プロンプトキャッシュが効いているか（`usage.prompt_tokens_details.cached_tokens`）を配信の長さで確認する
- [ ] 翻訳の質: 英語・ロシア語・中国語以外の言語で、ゲーム実況の口語がどこまで自然に訳せるか
- [ ] ヒーローの `items` から signature スキルを引くときの対応関係

## 17. LP とサーバー（`web/`, `server/`）

どちらも pnpm workspace のパッケージで、Cloudflare Workers に置く。`deadlock.jp` には別の Worker がいるので、サブドメインを分け、Worker も分ける（LP の文言を直してもアプリの使う API に響かず、別々にデプロイできる）。

| | ドメイン | 中身 |
|---|---|---|
| `web/` | `jimaku.deadlock.jp` | Astro の静的なページ（トップ・プライバシーポリシー・404）を Workers の静的ファイル配信で配る。サーバーで動かす処理はないので Astro のサーバー機能は使わない |
| `server/` | `api.deadlock.jp` | Hono。パスはアプリごと（`/jimaku`）と版（`/v1`）で区切る（`deadlock.jp` の下でほかのものを作っても困らない） |

- **日本語だけ**: ドメインと利用者が日本向けで、ふりがなのように日本語に寄せた機能なので、LP もアプリの画面も日本語だけにする（英語に対応すると、文言を足すたびに両方書く手間が続く）
- **API**: `GET /jimaku/v1/aliases/{lang}` で別名（8.5）を返す（1 時間キャッシュ）。中身はリポジトリの `data/aliases.*.json` をビルド時に入れる。アプリは起動時に取りに行き、取れなければ同梱のものを使う
- **LP**: ゲームの画像は載せない（字幕の見本は文字だけで組む）。フッターに Valve の公式ではないことと、ゲームのデータの出どころ（deadlock-api）を書く。プライバシーポリシーには、OpenAI・deadlock-api・api.deadlock.jp に送るものと、PC の中に保存するものを書く
- **デプロイ**: 手元から `pnpm --filter ./web deploy` / `pnpm --filter ./server deploy`（先に `wrangler login`）。カスタムドメインは各 `wrangler.jsonc` の `routes`（`custom_domain: true`）で付く
- サーバーは Hono（最初から Cloudflare Workers 向け）。ElysiaJS も考えたが、Workers 対応が実験的なので選ばない

