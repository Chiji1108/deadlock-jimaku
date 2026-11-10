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
- 音声認識: OpenAI（GPT-Live-Transcribe と GPT-Transcribe）か xAI（Grok Voice Transcribe。確定だけ）。3 章
- 翻訳と辞書の漢字名の読み: OpenAI GPT-6.1 Terra（推論なし。9.3）
- ふりがな: GPT-6.1 Terra（文ごと。答えは確かめてから使う）、固有名詞: 翻訳と同じ問い合わせで GPT-6.1 Terra に辞書の用語から選ばせる（9 章）
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
- Realtime は用語が 50 件・20 文字までで、公式名も渡しきれない。区切りはサーバーの VAD（無音 0.75 秒）と最後の手動 commit で、アプリの区切りとは同じでない（用語なし / ありで区切りの数も違った）
- 最初の partial が出るまで（接続から）はどちらも約 2.5 秒で差がない

**xAI と Qwen の final も比べた**（2026-11、`experiments/stt_eval_xai_qwen.py`。各 3 回。用語は辞書の日本語の公式名）:

| | sample01 | sample02 | 1 時間あたり | 応答 |
|---|---|---|---|---|
| Grok Voice Transcribe 2.0（用語なし / ありは先頭の 100 件） | 1.6〜4.0% / 1.6〜4.8% | 5.9% / 5.9%（拡マガ → 角魔蛾） | $0.10 | 0.6〜1.0 秒 |
| Qwen3-ASR-Flash（用語なし / 393 件を文脈に） | 4.0% / 4.0% | 2.0% / 2.0% | $0.126 | 1.4〜2.5 秒 |
| Qwen-Audio-3.1-ASR-Flash（用語なし / 400 文字まで文脈に） | 2.4〜3.2% / 3.2% | 4.9% / 3.9% | 約 $0.014（トークン課金。音声 1 秒あたり約 14 トークン） | 1.3〜3.1 秒 |
| （GPT-Transcribe、上の表） | 0.8% | 3.9% | $0.27 | 1.6〜2.3 秒 |

- どれも「拡マガ」を書けない（角魔蛾・角マガ）。辞書の公式名は「拡張マガジン」で、用語を渡しても略称は直らない
- Grok は同じ音声でも回によって「強そうである」「強そうだよね」と揺れる。用語を渡しても良くならない。いちばん速い
- Qwen3-ASR-Flash は sample02 でいちばん良いが、sample01 で「いやー楽しみだなー」を「いや楽しみだな」、「強そうではある」を「強そうだよね」と言い換える。用語を渡すと読点が消える
- Qwen-Audio-3.1-ASR-Flash は OpenAI 互換の口では使えず、QwenCloud 独自の口（`/api/v1/services/aigc/multimodal-generation/generation`）で送る。単語の間に空白を入れ、「えーね」を「えね」、「取りたい」を「撮りたい」と書く
- Qwen の音声認識はシンガポールの鍵でしか使えない（東京の鍵では `model_not_found`）。翻訳と用語の Qwen3.8 は東京でも使える
- 録音 2 本だけなので差は小さく、どれも GPT-Transcribe を確かに上回るとは言えない。値段は Grok と Qwen3-ASR-Flash が約半分、Qwen-Audio-3.1 が約 20 分の 1

**partial の候補として Qwen-Audio-3.1-ASR-Flash-Streaming も測った**（2026-11、`experiments/stt_eval_qwen_streaming.py`。各 3 回、100ms ずつ実時間で送る。用語なし）:

| | sample01 | sample02 | 話した 1 分 | 最初の partial（送り始めから） |
|---|---|---|---|---|
| Qwen-Audio-3.1-ASR-Flash-Streaming | 2.4〜4.8% | 2.9〜4.9% | 約 $0.0008（トークン課金。音声 1 秒あたり入力約 11 トークン） | 1.8〜2.6 秒 |
| （GPT-Live-Transcribe、用語あり。上の表） | 4.0% | 4.9% | $0.017 | 2.3〜2.6 秒（接続から） |

- 精度は GPT-Live-Transcribe と同じか少し良く、値段は約 20 分の 1（話している 26 分で 1 時間あたり約 $0.02）
- 口は QwenCloud 独自の WebSocket（`wss://ap-southeast-1.maas.qwencloudapi.com/api-ws/v1/inference`、`run-task` → 音声のバイナリ → `finish-task`）。接続から `task-started` まで約 0.4〜0.7 秒
- 文の区切りはサーバーの VAD が決める（`sentence_id` と `sentence_end`）。1 本の録音が 3〜4 文に分かれ、アプリの区切りとは同じでない。最後の文は音声の終わりから約 0.3 秒で確定する（`finish-task` から `task-finished` までは約 10 秒かかる）
- 「取りたい」を「撮りたい」と書く回があり、回によって語尾が揺れる（「強そうである」「強そうだよね」）
- 用語（ホットワード）の渡し方はこのモデルの例に載っておらず、今回は試していない

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
- 確定の文字起こしを Grok にしたら、同じ WAV を xAI の `https://api.x.ai/v1/stt` に送る（`Access` の会社が xAI。`stt::transcribe`）。multipart で `model`（`grok-voice-transcribe-2.0`）, `language`, `keyterm`（同じ用語の先頭から、50 文字までのものを 100 件）, `file`（ファイルより後ろの欄は読まれないことがあるので最後）。録音の説明（prompt）は渡せない。鍵は自分の xAI の API キーか、xAI のキーを預けたおすそ分けのギフトコード（17.1）。間違った鍵には 401 ではなく 400 と「Incorrect API key provided」が返る（実測）ので、その 400 を 401 とみなして OpenAI と同じく `share::refusal` に渡し、字幕を止める（自分のキーなら `InvalidKey { provider: xai }`、ギフトコードなら `GiftUnknown`）。料金は 1 時間 $0.10

### 3.3 1 時間あたりのコスト見積もり

| 項目 | 見積もり |
|---|---|
| Realtime（話している間だけ送る。仮に 40% + 発話ごとの前後） | 約 26 分 × $0.017 = **約 $0.44** |
| GPT-Transcribe（発話区間のみ） | 約 26 分 × $0.0045 = **約 $0.12** |
| 翻訳（GPT-6.1 Terra。1 時間に約 600 発話） | 翻訳先 1 言語で **約 $0.3**、1 言語増えるごとに **+約 $0.3** |
| ふりがな（GPT-6.1 Terra。漢字を含む発話だけ） | **約 $0.3** |
| 合計（翻訳先が英語のみ、ふりがなあり） | **約 $1.2/時間**（話し続けると約 $2。下の実測） |

ライトのモード（確定は Grok、翻訳と用語は GPT-6 Luna、ふりがなは PC の中）は、Grok が約 26 分 × $0.0017 = 約 $0.04、Luna の翻訳が Terra の約 6 分の 1 で約 $0.05、合わせて **約 $0.1/時間**（推定。アプリでは測っていない）

「話している途中から字幕を出す」をオフにすると Realtime につながず、確定の字幕だけを出す。途中の字幕の分がなくなり、sample02 では料金が約 46% 下がった（字幕は話し終えてから出るので、長い発話ほど最初の字幕が遅くなる。sample02 の最初の発話で 6.5 秒が 11.3 秒に）。

アプリで数えた実測（sample02、29 秒・5 発話）: 途中の字幕 $0.0058（21 秒）、確定の字幕 $0.0015（20 秒）、翻訳 $0.0024（1,771 トークン）、ふりがな $0.0026（1,333 トークン）、合計 $0.012。話し続けた 1 時間に直すと、sample01 で約 $1.9、sample02 で約 $1.5。LP には「話し続けて約 $2」と書く。操作パネルの「料金」タブに出す（13 章）。

「話した言葉を出す」をオフにすると、途中の字幕とふりがなが動かない。sample02 の実測で $0.0141 が $0.0062 になり、約 56% 下がった（最初の字幕は 2.7 秒が 15.0 秒に。最初の長い発話を話し終えて、翻訳が届くまで出ないため）。1 時間の目安は約 $0.5。

Voxtral のときはリアルタイムに無音も含めて送り続けていた（delta を届いた時刻で発話に振り分けていたので、途中を抜けなかった）。OpenAI では発話ごとに commit するので、話していない間は送らない。

## 4. リポジトリ構成

```
deadlock-jimaku/
├─ DESIGN.md
├─ Cargo.toml                 # Rust の workspace（crates/core と app）
├─ package.json               # pnpm の workspace（frontend・web・server）
├─ pnpm-workspace.yaml
├─ crates/
│  ├─ core/                   # Tauri に依存しない。単体でヘッドレス起動・テストできる
│  │  └─ src/
│  │     ├─ audio.rs          # マイク → 16kHz mono フレーム + リングバッファ
│  │     ├─ stt.rs            # OpenAI の realtime / batch クライアント
│  │     ├─ enrich.rs         # 翻訳（OpenAI）の呼び出し・出力の検証
│  │     ├─ subtitles.rs      # パイプライン（Segment の状態遷移）
│  │     ├─ chrome.rs         # 聞き取りのページを Chrome で開く
│  │     ├─ annotate.rs       # 読みと用語を付ける（専用のスレッド）
│  │     ├─ reading.rs        # 形態素解析（Lindera + IPADIC）
│  │     ├─ terms.rs          # 速い用語の判定
│  │     ├─ assets/           # deadlock-api → glossary + アイコンのキャッシュ
│  │     ├─ rank.rs           # ランクのポーリング
│  │     ├─ lang.rs           # 言語の対応表
│  │     ├─ store.rs          # 状態ストア + broadcast
│  │     ├─ server.rs         # axum のルーティング
│  │     └─ types.rs          # ts-rs で TS に出力する型
│     └─ src/
│        ├─ main.rs           # 音声認識・翻訳と、アプリとの標準入出力
│        ├─ furigana.rs       # 形態素解析とふりがな
│        ├─ terms.rs          # 用語の判定
│        └─ models.rs         # モデルのダウンロードと検証
├─ app/                       # Tauri シェル（トレイ、keyring、自動更新、core の起動）
├─ data/                      # 手で管理する辞書データ（git 管理）
├─ notices/                   # 第三者のライセンス表示（13.7）
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
  subtitleStyle: SubtitleStyle;    // 字幕の文字の見た目（11.1）
  rankStyle: RankStyle;            // ランクの文字の見た目（11.1）
  rankLanguage?: LangCode;         // ランク名の言語。なければ話す言語に合わせる。新しく使い始める人は en（設定ファイルがないときだけ。保存済みの値は変えない）
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
- 標準のフォントは Windows のシステムフォントを優先し、同梱はしない。

```css
[lang]         { --font: initial; font-family: var(--font,) "Noto Sans", "Segoe UI", system-ui, sans-serif; }
:lang(ja)      { font-family: var(--font,) "Noto Sans JP", "Yu Gothic UI", "Meiryo", sans-serif; }
:lang(zh-Hans) { font-family: var(--font,) "Noto Sans SC", "Microsoft YaHei UI", sans-serif; }
:lang(zh-Hant) { font-family: var(--font,) "Noto Sans TC", "Microsoft JhengHei UI", sans-serif; }
:lang(ko)      { font-family: var(--font,) "Noto Sans KR", "Malgun Gothic", sans-serif; }
:lang(th)      { font-family: var(--font,) "Noto Sans Thai", "Leelawadee UI", sans-serif; }
```

- 字幕のフォントは言語ごとに選ぶ（設定の `subtitleStyle.languages.ja.font`）。キーは 6.2 の言語コードで、キーがない言語は標準のまま。操作パネルの字幕タブに、話す言語と翻訳先の行が並ぶ。
- ランクのフォントは 1 つで（設定の `rankStyle.font`）、なければ標準。操作パネルのランクタブの 1 行で選び、ランク名の言語にかかわらず全部の文字に当てる。字幕とランクは別々に選ぶので、片方の変更は他方に効かない。アイテム説明のカードはゲームと同じ見た目にするので対象外。
- 保存するのはファミリー名と、太さ、字間（`FontStyle`、11.1）。変更は JSON Merge Patch で、字幕は言語ごとの項目単位で重ね、字幕もランクも `null` で標準に戻す。知らない言語コード、256 文字を超える名前、制御文字を含む名前は変更なら拒否し、設定ファイルならその項目だけ捨てる。PC にあるかは確かめない。
- v0.3.2 の帯（`subtitleDisplay.background`）とフォント（`overlayFonts`）は移さない。使った人がほぼいないので、知らない項目として捨て、標準に戻す。ほかの設定は残る。
- 選んだフォントは `display` で届く。字幕は `subtitleStyle.languages`、ランクは `rankStyle.font` で、別々の項目にしているので、字幕の変更でランクの規則は変わらない。`Captions` と `RankBadge` は、`lang` 属性を持つ要素（字幕の行、ランクの根の要素）に `--font: "名前",` を直に書く（`overlay-fonts.ts` の `fontVar`。`"` と `\` はエスケープする）。字幕は行の言語のキーで `languages` を引くので、`es` の設定が `es-419` の行に当たらない。ランクは 1 つのフォントを根に書き、ランク名とポイントを同じフォントで描く。LP の見本も同じ部品に同じ値を渡すだけでフォントが当たる。`Subtitles.svelte` と `Rank.svelte` は自分のページの 1 つの `<style>` に、選んだフォントごとの `@import url("/api/fonts/名前/faces.css");` だけを入れる。`display` はフォント以外の変更でも届き、`@import` を書き直すと読み終えるまで一瞬標準に戻るので、中身が同じならシートを書き直さない。末尾のカンマで標準の候補につながるので、PC にないフォントは標準に落ちる。`--font` も継承されるので、`lang` 属性を持つ要素ごとに `initial` へ戻す（直に書いた値はこれより強い）。これで `<html lang="ja">` のフォントが `lang="en"` の子に移らず、選んでいなければ `var()` の空の代替値で標準だけになる。
- 選べる一覧は `GET /api/fonts`（`fonts.rs`）。`fontdb` で PC のフォントを読み、英語のファミリー名に、下の Google Fonts のフォントを重ねた `{ family, languages?, sizeBytes?, downloaded }` の配列を、カタログの順の Google Fonts のフォント、名前順の PC のフォントの順に返す。`languages` があるものは Google Fonts のフォント、`sizeBytes` があるものは未取得、`downloaded` は PC になくダウンロードで置いたもの。`.` で始まる macOS の内部用の名前と、設定に保存できない名前は除く。呼ばれるたびに `fontdb` を読み直す（下の配信もこの `fontdb` を使う）。操作パネルは起動したとき（`Panel.start`）と、フォントの行を出したときと、フォントを選ぶダイアログを開いたときに読み、同時には 1 回だけ走らせる。読み直しの間と失敗したときは、前の一覧を出したままにする。ダウンロードのあとは、走行中の読み込みが終わるのを待って読み直す。これで、あとから入れたフォントが起動し直さなくても一覧に出る。
- 名前の指定だけでは、あとから入れたフォントは描けない。WebView と OBS は起動時のフォントの情報を持ち続けるので、core がファイルを配って `@font-face` で読む（Windows と OBS で、再起動なしに描けることを確認した）。一覧の見本は名前で描くので、最初の一覧のあとに増えたフォントと、`downloaded` のフォントだけ `faces.css` を読み込む。最初の一覧は操作パネルの起動時に読むので、WebView が起動した時点のフォントとそろう（フォントの行を初めて出すまで待つと、その間に入れたフォントを描けるものと取り違える）
- 配信はどちらも `/api` の下で、`local_only` の内側。通信は `127.0.0.1` の中だけで、ファイルは外に出ない。
  - `GET /api/fonts/{family}/faces.css`: そのファミリーの直立で標準幅の面を `@font-face` で書く。そういう面がないファミリー（Impact、Arial Narrow、Condensed だけのフォント）は、全部の面を候補にする。`@font-face` は太さしか書かず、同じ太さの規則は後が前を隠すので、太さごとに 1 面だけを残す。直立の面を優先し、次に PostScript 名の短い面、同点は名前順で、読み込み順によらず、通常の面が Bold や Ornaments に隠れない。`font-weight` は実際の値で、可変フォントは `fvar` の `wght` の範囲。`font-display: swap`。`font-family` は選んだ名前のままなので、`--font` の規則は変わらない。面がなければ 404
  - `GET /api/fonts/{family}/{postscript}`: 1 面の sfnt。`faces.css` の `src` はこの相対の URL
- `fontdb` は `Store` の `fonts::Library` が 1 つ持ち、読み直すのは `GET /api/fonts` とダウンロードを終えたときだけ（一覧をまだ読んでいなければ、初めて配るときに読む）。リクエストごとに PC の全部のフォントを読むと 1 回 6ms ほどかかり、並べて読むと重なって遅くなるため。一覧に出るファミリーの、名前と PostScript 名が一致した面だけを返す。URL の値は `fontdb` の結果と比べるだけで、ファイルパスを作らないので、任意のファイルは読めない。どちらも `no-cache` に `ETag` を付け、変わっていなければ `If-None-Match` に 304 を返す。面の `ETag` はファイルの大きさと更新時刻と面の index、`faces.css` は内容のハッシュ。フォントを入れ替えれば同じ URL で新しい面が届く。
- `.ttc` と `.otc` は 1 面の sfnt に組み直して返す。Chromium は `url(...#1)` の面の指定を無視する。それ以外のフォントはファイルの中身をそのまま返す。どちらも 1 面をメモリに載せて返す（ファイルは `fontdb` と同じくマップするので、コレクション全体は載らない）。ローカルの配信で、ブラウザは描く面しか取りに来ないので、大きさの上限は置かない。
- ファイルが読めないとき（OTS に拒否される、404）は、ブラウザが同じ名前の `font-family` を PC のフォントとして探し、なければ標準の候補へ進む。なので `src` に `local()` は足さない。macOS の Courier は OTS に拒否されるが、この経路で描ける
- Google Fonts のフォントは同梱せず、選んだ人だけがダウンロードする。
  - カタログは `crates/core/src/font-catalog.json`。`google/fonts` のコミットを固定し、ファミリー名、言語、見本の文字、ファイルごとの名前と大きさと SHA-256、ライセンスを持つ。OFL だけを載せる。日本語と英語は上限なしで、ラテン文字、キリル文字、ギリシャ文字の言語には文字を持つフォントだけが出る。韓国語、中国語、タイ語は 3〜5 種類を目安に並べる。ハングルのフォントは大きいので、ほかの言語の行に出さず、韓国語の枠に入らないものは載せない（Hi Melody、Gamja Flower、Poor Story）。並びはカタログの順で、系統でまとめる（丸い、かわいい、手書き、ポップ、太い、レトロ、上品、本文）。Noto Sans JP や Noto Sans のような本文向けが最後に来る。ファミリー名はファイルの中の名前（`Rounded Mplus 1c` は Google Fonts の名前と違う）。`languages` は勧める言語で、文字を持つ言語の全部ではない
  - `POST /api/fonts/{family}/download`（`local_only` の内側）はカタログの名前だけを受ける。URL は受け取らない。ファイルごとに `res.chunk()` で受け、カタログの大きさを超えたら止め、SHA-256 を確かめる。全部のファイルが揃ってから `write_atomic` で `data_dir/fonts` に置く。全部のファイルを受け終えるまで 1 回 10 分までで、全体の時間は `download_from` の 1 か所で切る。`http()` の 60 秒を持たない専用のクライアントを使う。次のデータが 30 秒届かなければ止める。同じファミリーの同時実行は 409
  - `data_dir/fonts` は PC のフォントと同じ `fontdb` に読むので、`faces.css` と面の配信はそのまま使う。ダウンロードを終えたら読み直すので、一覧を読み直す前から配れる。一覧の PC のフォントには数えない（ファイルが `data_dir/fonts` の下の面は除く）。一覧の未取得の判定はファイルの有無で、書き込みの途中で失敗して欠けたファミリーは未取得のまま出て取り直せる
  - PC に同じ名前があれば PC を優先し、未取得として出さない。削除はまだない
  - 字のあるフォントだけを言語に載せる。`mise run font-samples` がファイルの中の名前と、言語ごとの文字が全部あることを確かめて、失敗する
- 字幕タブとランクタブの各行は同じ部品（`FontPicker.svelte`）で、名前（字幕は言語名、ランクは「ランク」）、今のフォント名、見本、「変更」のボタンだけを出し、ボタンでダイアログを開く。ランクのダイアログはランクの言語（`rankLanguage ?? micLanguage`）で絞る。ダイアログは上に検索欄、続けて「標準」、見出し「Google Fonts」にその行の言語のカタログのフォント、見出し「この PC のフォント」にそれ以外を名前順に並べる。Google Fonts のフォントはカタログの順。ほかの言語のものは、取得済みか PC にあれば「この PC のフォント」に出し、未取得なら出さない。検索は全部を通して絞る。各フォントは見本（28px）と名前のカードで、ボタンにする。広い画面は 2 列、狭い画面は 1 列。矢印キーで検索欄とカードを移り、検索語があれば Enter で先頭のカードを押す。取得済みと PC のフォントは押すとすぐ適用して閉じる。未取得のフォントを押すと `ConfirmDialog` で大きさを確かめる。やめれば一覧に戻り、進めたらダイアログを閉じて裏でダウンロードする。その間はトーストとカードに進行中を出し、タブを移っても続く。成功したら適用する。待つ間にその言語のフォントを選び直していたら、選び直したものを残す。失敗はトーストで出す。
- 未取得のカードの見本は、文字をパスにした SVG（`frontend/src/control/font-samples/`）。`mise run font-samples` が `fontkit` で作ってコミットし、アプリに同梱する。`currentColor` で描くので、`?raw` で読んでインラインに置く。合わせて 350 KB あるので操作パネルの初めの読み込みに入れず、ダイアログを初めて開いたときに 1 つのチャンクで読む。読み終えるまでカードは名前と大きさだけで、押して選べる。見本の文字はそのフォントの主な言語（日本語、한국어、简体中文、繁體中文、ไทย、Latin は Abc）。em ではなく実際の字の高さで大きさを揃えるので、字の小さいドットフォントも潰れず、高い字も切れない。実行時の通信はなく、ダイアログを開いても外に接続しない。
- 取得済みのカードは、名前を UI のフォントで、見本をそのフォントで描く。カードの見本も行の見本も、その言語での言語名（日本語だけ、かなと漢字を混ぜた文字列）（`Intl.DisplayNames([lang], { type: "language" }).of(lang)`）を使う。見本の `font-family` は標準の候補につなげない。画面外のカードは `content-visibility: auto` で描かない。

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

アプリの起動時に `client-versions` の最新を確認し、その版のキャッシュがなければ作る。トレイに常駐すると起動し直さないので、字幕を始めるときにも裏で確かめる（`assets::watch_versions`）。字幕の開始は待たせず、新しい版があれば今の処理で作り直して差し替え、`glossary.updated` を流す。用語の表も辞書の変更に合わせて作り直す。確認に失敗したら今の辞書で続ける。

1. **`mic.language`・`en`・`translateTo` の言語だけ**を取得し、`id` で結合する（アイテム一覧は 1 言語で約 7MB あるので、29 言語をまとめて取らない）。どのエンティティを載せるかは英語版で決める（英語名がないものは載せない）。
2. アイコンを `{データディレクトリ}/assets/{形式}/{clientVersion}/icons/{kind}/{id}.webp` にダウンロードし、`/assets/{clientVersion}/icons/...` で配信する（配信中に外部へ依存しない。URL に版を含むので `immutable` で長くキャッシュできる）。ファイルは一時ファイルに書いてから置き換える。取れなかったアイコンはそのエンティティから外して表示せず、辞書も保存しない（次の起動で作り直し、欠けた分だけ取り直す）。
3. **辞書の漢字名の読みは LLM（`models.readings`、既定 `gpt-6.1-terra`、推論なし）に読ませ、名前ごとにキャッシュする**（`assets/readings.rs`）
   - 辞書は読みなしで保存し、読み込むたびにキャッシュ（`{データディレクトリ}/assets/readings-llm.ja.json`）の LLM の読みを重ねて配信する。キャッシュにない名前は配信した後に 40 件ずつ問い合わせ、届いたら配信し直す
   - 辞書の保存と切り離しているので、キーが無効・未設定でも辞書は保存され（読みなしで動く）、キーを後から設定すれば次の起動で効く。問い合わせが失敗したら次の起動で問い直す
   - 答えは、つなげると名前になること・読みがひらがなであること（カタカナはひらがなにする）・漢字には必ず読みがあることを確かめ、読みを漢字の部分ごとに割り当てる（かなの部分を読みの中で探して区切る。「取り扱い」→ 取(と)り扱(あつか)い、「魔呪の罠」→ 魔呪(まじゅ)の罠(わな)。字幕の文のふりがなにも同じ確かめ方を使う）。通らなかった名前はキャッシュせず、次の起動で問い直す（答えは揺れるので、たいてい次は通る）
   - キャッシュが壊れていたら `.broken` に退けて空から始める。読めない（権限など）ときは LLM の読みを使わない（黙って上書きしない）
   - client 6839: 漢字を含む名前 35 件すべてに正しい読みが付いた（人狼・屍鬼累々・魔瓶・聖別手榴弾・五芒星・魔呪・一意専心・餌付け・袋叩き など）。初回の問い合わせは約 19 秒、2 回目以降はキャッシュから
   - 比較（35 件、判断が難しい 10 件で採点）: GPT-6.1 Terra（推論なし）と Claude Sonnet 5.5（推論あり low）が 0 件、Sonnet 5.5（推論なし）1 件、Mistral Large 4 2 件（一意→いつい、餌付け→えさつけ）、Haiku 5.5 2〜3 件。形態素解析（IPADIC）は造語 7 件を誤る（知らない複合語を 1 文字ずつ訓読みする。「人(ひと)狼(おおかみ)」）
   - キーがなければ LLM の読みはない。用語の判定は Lindera + IPADIC が名前を読んだ結果を使う（9.6）。手で保守する読みは持たない
   - 推論の深さは `models.reasoning_effort`（既定 `none`。推論しないモデルにするときは未設定にしてパラメータを送らない）

- キャッシュに必要な言語が揃っていれば作り直さない（2 回目以降の起動は 1 秒未満）。初回の生成は 3 言語で約 8 秒。`languages` には実際に名前が取れた言語だけを載せる。
- `{形式}`（`CACHE_FORMAT`）は Glossary の形やアイコンの置き方を変えたら上げ、古いキャッシュを読まないようにする。
- 同じスキルを複数のヒーローが持つ場合は、最初のヒーローのものとして 1 件だけ載せる。
- 通信できないときは、手元にある最新の版を使う（言語が足りなくても使う）。
- データディレクトリは Tauri の `app_data_dir`（ヘッドレスでは `dirs::data_dir()/jp.deadlock.jimaku`。同じ場所）。
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
      "icon": "/assets/6839/icons/hero/13.webp"
    },
    "item:1548066885": {
      "kind": "item",
      "className": "upgrade_clip_size",
      "names": { "en": "Extended Magazine", "ja": "基礎拡張マガジン", "ru": "Увеличенный магазин" },
      "ruby": [{ "text": "基礎拡張", "ruby": "きそかくちょう" }, { "text": "マガジン" }],
      "icon": "/assets/6839/icons/item/1548066885.webp",
      "slot": "weapon"
    },
    "ability:731943444": {
      "kind": "ability",
      "className": "ability_bullet_flurry",
      "names": { "en": "Bullet Dance", "ja": "バレットダンス", "ru": "Танец пуль" },
      "icon": "/assets/6839/icons/ability/731943444.webp",
      "hero": "hero:13"
    }
  }
}
```

- エンティティの参照は `"{kind}:{id}"`（`hero:13`, `item:…`, `ability:…`）で統一する。
- `ruby` は日本語の名前の読み（`names.ja` 用）。漢字を含む名前だけに付ける。
- `slot`（`weapon` / `vitality` / `spirit`）はアイテムの色分けに使う。
- 訳文では、発話された略称ではなく公式名を出す（`拡マガ` → `Extended Magazine`）。辞書にない言語の名前は英語名で出す。

### 8.4 音声認識への用語ヒント

`keywords` には、`mic.language` での公式名を入れる（上限 500 件。送りすぎない。ヒーロー → アイテム → スキルの順に残す）。略称や言い間違いは、音声認識ではなく用語の判定（9 章）の側で公式名に結びつける。

### 8.5 スキルの番号（`abilitySlot`）

辞書のスキルに、持ち主の何番目のスキルか（`abilitySlot`。API の `signature1`〜`signature4`。4 がウルト）を入れる。用語の判定（9 章）で LLM に渡し、「ヘイズのウルト」「グレイブスの3番」「ドアマンのカート」のように、番号や名前の一部で呼んだスキルを決めさせる。

- 原文では話した言葉のまま（ウルト）にスキルのアイコンと色を付ける。翻訳では公式名になる（Haze's [アイコン] Bullet Dance）。略称も同じ（11 章）
- 以前は手で集めた別名と、書き方の揺れを揃える処理、「ウルト」「N番スキル」「ヒーロー名の後ろのスキル名の一部」を探す処理で照合していた。LLM に任せたら（9.1）どれも要らなくなったので消した。LLM を使わない判定（9.6）も別名を持たず、公式名だけを見る

## 9. enrich（`enrich.rs`, `furigana.rs`）

### 9.1 方針: 用語の判定と翻訳を LLM に任せ、答えは確かめてから使う

最初は「トークン分け・ふりがな・固有名詞・翻訳・雑音の判定」をすべて LLM（Mistral Small 4）に返させたが、問題が出たので役割を分けた。その後、モデルを GPT-6.1 Terra にして、用語の判定を LLM に戻した（16 章）。

| 問題 | 対応 |
|---|---|
| ふりがながずれる（`僕(ぼ)`、`最(さい)近(き)`）、送り仮名ごと付ける（`強い(つよい)`）（Small 4） | 読みは**語ごとの区切りと読み**だけを返させ、つなげると原文になること・読みがひらがなであることを確かめ、送り仮名を外す割り当てはコードでする（GPT-6.1 Terra。形態素解析より造語・文脈に強い）。初めは形態素解析（Lindera + IPADIC）にしたが、文脈で決まる読み（上方修正・金・一人）を誤るのでやめた |
| 名前の途中で区切る（`ヘ⟨イズ⟩`）、同じ名前を付けたり付けなかったりする（Small 4） | 一度は辞書の名前と別名の最長一致にした。しかし略称・言い間違い（リコセ）・番号で呼んだスキル（グレイブスの3番）は別名を手で足し続けても拾いきれず、「ページ」を Paige にするような誤りも出た。GPT-6.1 Terra に辞書の公式名だけを渡して判定させたら、37 文 × 3 回で正しく、答えのぶれも区切りの崩れもなかった（`experiments/results/entity_llm.json`）ので、LLM に戻した |
| 普通の文を「雑音」と判定して消す（Small 4） | 判定しない。咳やフィラーだけの音声はバッチの文字起こしが空になり、そこで捨てる |
| 訳文で記号を使わず名前をそのまま書く（`Haze's`）（Small 4） | 訳文の用語は `⟦hero:13⟧` のように参照で書かせ、各言語の公式名は描画時に辞書から出す |

実験（`gpt-6.1-terra`、推論なし、client 6842）: 辞書の公式名だけ（別名なし）で、次をすべて正しく判定した。

- 略称・言い間違い・英語名のカタカナ読み: 拡マガ、エコシャ、カース、リコセ、リコシェット、ページ（Paige の聞き違い。「ページのウルト」）
- 番号や名前の一部で呼んだスキル: ヘイズのウルト、ヘイズの2、グレイブスの３番、ドアマンのカート、ドアマンカート（「の」なし）、前の発話がドアマンの「カートに気をつけて」
- 用語にしないもの: 1番強いキャラ、次のページを見て、コンビニでカート、チャージ、マーク、ソウル、ミッド、ガーディアン、ヒーロー名のない「ウルト溜まった」、スマホはポケットに、セブンイレブン、最初からラッシュしよう（「ポケットのウルト」「セブン止めないと」「ラッシュに飛び込まれた」はヒーローにする）
- 返答は 1.6〜3.8 秒（平均 2.3 秒）。用語の一覧で入力が約 9,700 トークン増えるが、system の先頭に固定で置くのでほぼキャッシュから読まれる（1 発話あたり約 0.0005 ドル増える）

### 9.2 処理の流れ

1. 日本語で漢字を含む文なら（ふりがなを出す設定のときだけ）、文全体の読みを LLM（`models.readings`）に聞く（2 と同時に）。直前 3 発話を文脈に付ける。答えは 8.2 と同じく確かめて漢字の部分ごとに割り当てる。答えが通らなければふりがななし
2. 字幕と直前 3 発話を LLM（`models.translate`）に渡し、字幕を「ふつうの文字」と「用語（辞書の参照）」の区切りに分けたものと、翻訳先ごとの訳文（用語は `⟦参照⟧`）を返させる
3. 区切りをつなげて字幕にならなければ、どこが用語か分からないので原文に用語を付けない。辞書にない参照はふつうの文字にする
4. 訳文の `⟦参照⟧` をエンティティに戻す。戻すのは 3 で原文の区切りに見つけた用語だけ（原文にない用語を訳文にだけ出さない）。ほかの参照は捨て、空いた空白を詰める（後ろの 's も）。同じ用語が続く区切り（「基礎」「拡張マガジン」）は 1 つにまとめる
5. 1 の読みを文字の位置で区切りに当てはめる。用語の中に収まる読みは用語のルビ（`Token` の `ruby`。「一番(いちばん)スキル」）、区切りをまたぐ読みは付けない

- system（役割・ルール・用語の一覧）は辞書の版と言語が変わったら作り直す（辞書が後から届いても、パッチで用語が増えても、言語を足しても効く）
- 翻訳先がなくても、辞書があれば用語のために 2 を聞く（訳文は聞かない）。辞書も翻訳先もなければ聞かない
- 2 に失敗したら、原文（とふりがな）だけで enriched にする
- デッドロックモードをオフにすると、用語は判定せず、1 と 2 のプロンプトからゲームの前提も外す（2 は翻訳だけを聞く。翻訳先がなければ聞かない）。文字起こしにも用語ヒントを渡さず、`PLAIN_PROMPT` を使う

### 9.3 LLM の入出力

- モデル: OpenAI `gpt-6.1-terra`（設定の `models.translate`、`reasoning_effort: none`）。速さより訳の質を優先した（確定字幕から翻訳まで 1.6〜3.8 秒）
  - Mistral Large 4 は推論を止められず（`reasoning_effort` は low / medium / high のみ）、1 発話に 21〜24 秒かかり、出力が途中で切れることもあったので使わない
  - Z.ai GLM 5.3 Flash（`zai-glm-5-3-flash`）も推論するモデルで、1 発話 5.5〜23 秒。訳の質は Small 4 とほぼ同じ。ふりがなを付けさせると普通の文は正しいが 20 秒かかり、造語の多い文では原文と無関係な文を作った
  - Small 4 は 1 発話 0.8〜1.8 秒、記号は 4/4 で保たれた
  - OpenAI `gpt-6-luna`（`reasoning_effort: none`）は 1.1〜3.4 秒。ロシア語訳に日本語の「実況」が残った
    - 2026-11 にアプリと同じ問い合わせで測り直した（`crates/core/examples/terms_eval.rs`、`experiments/results/terms_eval.json`。9.1 の 37 文 × 3 回、訳は英語とロシア語）。用語の判定は Terra の 106/111（外れた 3 回のうち 2 回は「ヘイズの2で寝かせてからウルト」のウルトも用語にしたもので、むしろ正しい。ほかに 1 回タイムアウト）に対して Luna は 97/111。Luna は「〇〇のウルト」のウルトを落とす（ページのウルト 3/3）、「のウルト」と助詞ごと区切る（4 回）、区切りが原文とつながらず用語がすべて落ちる（ヘイズの2… 2/3）、ふつうの言葉を用語にする（スマホはポケットに 1/3）。訳文に日本語が残ったのはどちらも 0 回。訳はどちらも自然で、Luna は「⟦Haze⟧'s ult」のようにスキルを参照にせず書くことがある。応答の中央値は Terra 1.9 秒、Luna 1.7 秒。料金は 1 発話あたり Terra 約 $0.0010、Luna 約 $0.00017（単価から。入力のほとんどがキャッシュから読まれる）
    - この結果から、翻訳と用語の選択肢に Luna を足した（安いが用語の判定は弱い、と添える）。おすすめは Terra のまま。ふりがなに Luna を使えるかは測っていない
    - Claude Haiku 5.5 も同じ問い合わせ（`Enricher::question` を Messages API の構造化出力で送り、`Enricher::read` で確かめる。推論は止め、effort `low`）で測った。用語は 85/111（ほかに 1 回タイムアウト）。ほかのスキルや別のアイテムの参照を選ぶ（カース、グレイブスの３番、ページのウルト）、区切りが原文とつながらず用語がすべて落ちる（ヘイズのウルト 3/3、カースは最終盤に 2/3）、ふつうの言葉を用語にする（スマホはポケットに 3/3、ヒーロー名のないウルト 2/3、ソウル、ラッシュ）。訳は原文にない言葉を足す（「no question」）、用語を落とす（「⟦Haze⟧'s hits so hard」）、ロシア語に英語を残す（「Guardian слишком…」）。応答の中央値は 1.3 秒でいちばん速く、料金は 1 発話あたり約 $0.00022（測った usage から）。推論を入れた場合は測っていない
    - Qwen3.8-Flash と Qwen3.8-Max も同じ問い合わせ（QwenCloud シンガポールの OpenAI 互換の口に Chat Completions の構造化出力で送る。思考は止め、辞書の一覧は明示のキャッシュに置く）で測った。用語は Flash 83/111、Max 81/111 で、どちらも Haiku 並み。外れ方は Max も Flash もほぼ同じで、「〇〇のウルト」のウルトを落とす（ポケット・バイオレットはどちらも 3/3、ヘイズ・ページは Max 3/3）、「カース」を用語にしない、「ガーディアン硬すぎる」の守護者をアイテムにする。Max は「拡マガ」を別のアイテムにする（3/3）。訳文に日本語が残ったのはどちらも 0 回だが、**Flash は 222 文のうち 36 文で参照を `⟦…⟧` で囲まずに `[hero:67]` と書き、字幕にそのまま出る**（Max は 0 回。`rawReferencesInTranslation`）。応答の中央値は Flash 1.3 秒、Max 2.3 秒。料金は 1 発話あたり Flash 約 $0.00024、Max 約 $0.0031（測った usage から。キャッシュの読みは入力の 0.1 倍で数えた）。Flash は Luna と同じくらいの値段で用語は弱く、Max は Terra の 3 倍の値段で用語は弱いので、どちらも選択肢に足さない
  - OpenAI `gpt-6.1-terra`（同）は 1.7〜3.8 秒。所有格のルールを守り、ロシア語がいちばん自然 → **採用**
  - Claude Haiku 5.5（推論なし）2.4〜3.9 秒、Claude Sonnet 5.5（推論なし）2.3〜9.2 秒。Sonnet の訳は自然
- system: 役割とルールと用語の一覧（1 行に 1 つ。`hero:13 | hero | ヘイズ | Haze`、スキルは `| ability 4 of Haze (hero:13)` を足す。話す言語の名前と英語名）
  - 略称・聞き違い・言い間違い、番号・「ult」・名前の一部で呼んだスキルも用語にする。ふつうの言葉（1番強い、ページ、買い物のカート）は用語にしない。用語の区切りは名前の部分だけ（「グレイブスの3番」は「グレイブス」「の」「3番」）
  - 用語に語尾を付けない（ロシア語の格変化などで名前が崩れる）。英語の所有格 's だけは許す（名前の後ろに付くだけで崩れない）
  - 英語名が冠詞から始まる用語（The Doorman）の前に冠詞を付けない
- user: `{ "context": [直前の確定字幕], "transcript": "ヘイズのウルト強い" }`
- OpenAI の Responses API（`/v1/responses`。`instructions` に system、`input` に user）。Chat Completions も使えるが、OpenAI が新しく作るものに勧めているほうにした。前の応答を引き継いで使わないので `store: false`（OpenAI に応答を保存させない）
- 構造化出力（`text.format`: `json_schema`, `strict`）: `{ "segments": [{ "text": "ヘイズ", "ref": "hero:13" }, { "text": "の", "ref": null }, …], "translations": { "en": "⟦hero:13⟧'s ⟦ability:…⟧ is strong" } }`。`translations` のキーは設定の翻訳先から毎回作る（翻訳先がなければ `translations` を聞かない）
- OpenAI の API キーもギフトコードもなければ聞かない（原文だけ）
- タイムアウト 8 秒
- 用語と翻訳はストリームで受け、訳文を書きかけのうちから字幕に出す（`Segment.draft`）。答えの JSON は訳文を先に書かせる。sample01・02 の 8 発話の中央値で、訳文の最初の文字は順番を変えて 1.70 秒が 1.11 秒になり、全部を待つ 2.14 秒より約 1 秒早い。原文の区切りがまだ届かないので、書きかけの用語は辞書にある参照をそのまま出し、完成したら確かめ直す。閉じていない `⟦` からあとは出さない

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
- 日本語への翻訳（話す言語が日本語以外のとき）は、訳文にふりがなが付かない（訳文は文字列で返させているため）。必要になったら訳文にも 9.2 の 1 を通す

### 9.6 モードと Chrome（`engines`、`finish.rs`、`chrome.rs`、`annotate.rs`、`reading.rs`、`terms.rs`、`frontend/src/chrome/`）

字幕の作り方を 4 つから選ぶ。新しく使い始める人の既定は `chrome`（インストール直後からキーなしで字幕が出せる）で、おすすめは `standard`。Chrome のモードで自分のキーか有効なギフトコードを保存すると、モードの欄に「スタンダードにする」の提案を出し、押して切り替える（設定を黙って変えない）。提案は閉じられて、閉じたら保存して再び出さない（`hybridHintClosed`。モードの欄から選べるので困らない）。スタンダードを選んで鍵がない人は「キーなしで始める（Chrome）」で、これも押して切り替える。途中と確定の文字、翻訳、ふりがな、強調を何で作るかは、工程ごとの選択（`Engines`）と設定から `Settings::plan` が 1 か所で決める（README の表と同じ。テストに同じ値を書く）。Chrome が要るかもここから分かる。OpenAI のモードの途中の字幕にも読みと用語を付ける。

**工程ごとの選択（`engines`）:** 設定には、モードの名前ではなく、工程（途中の字幕・確定の文字起こし・翻訳と用語・ふりがな）ごとに何で作るかを持つ。モードは決まった組み合わせ（`Engines::CHROME`・`VALUE`・`HYBRID`・`OPENAI`）で、操作パネルの「詳しく選ぶ」のダイアログ（`EnginesDialog.svelte`）から工程ごとに変えられる。人それぞれの事情で値段と精度を選べ、料金の内訳も見え、OpenAI 以外のモデルも工程の選択肢として足せる。選んだ組み合わせがどのモードにも当たらなければ、モードの欄はどれも選ばず、OpenAI を使う工程を書き出す。ふだんの人は 4 つのモードから選ぶだけで済むよう、工程の選択はダイアログに置く。0.4 までの設定ファイルは `load` が `speechEngine` と `liveSubtitles` から置き換える（0.3 より前の `speechEngine` のない設定ファイルは `openai` のまま。Chrome のウィンドウを黙って開かない）。鍵が要るかは工程の選択だけで決める（`Engines::paid`。OpenAI を使う工程が 1 つでもあれば要る）。翻訳先やふりがなの表示で決めると、表示を切り替えるたびに字幕の処理を始め直すことになるため。

| モード | 途中の字幕 | 確定の字幕・翻訳・用語 | 特徴 |
|---|---|---|---|
| `light`（ライト） | Chrome（Google）。途中の訳は Translator | 確定の文は xAI の Grok、翻訳と用語は GPT-6 Luna、読みは PC の中 | 高速で高精度。約 $0.1/時間。OpenAI と xAI の鍵が要る。音声は Google と xAI に送られる |
| `standard` | Chrome（Google）。途中の訳は Translator | OpenAI | 高速で、最後は最高精度。約 $0.6/時間。音声は Google と OpenAI に送られる |
| `openai` | GPT-Live-Transcribe | OpenAI | 低負荷で最高精度。Chrome を使わないので、途中の字幕は Chrome を使うモードより遅い |
| `chrome` | Chrome（Google） | 確定の文は Chrome、翻訳は Translator、読みと用語は PC の中 | 無料、API キー不要。音声は Google に送られる。翻訳は Chrome の中で動く |

| 工程 | 無料 | 有料 |
|---|---|---|
| 途中の字幕 | Chrome（Google）。途中の訳は Translator | GPT-Live-Transcribe（途中の訳なし）。「出さない」も選べる |
| 確定の文字起こし | Chrome（発話の区切りも Chrome が決める） | GPT-Transcribe か xAI の Grok Voice Transcribe（アプリのマイクと VAD で区切る。3.2） |
| 翻訳と用語 | Chrome の Translator と辞書の一致（`terms.rs`、日本語だけ） | GPT-6.1 Terra か GPT-6 Luna（9.3。選んだモデルを `Settings::models_in_use` が渡す） |
| ふりがな | Lindera（日本語だけ） | GPT-6.1 Terra |

**組み合わせの決まり:** 今の仕組みで作れない組み合わせは `Engines::sanitized` が直し、操作パネルでは選べなくする（`frontend/src/lib/engines.ts` の `unavailable`）。

- 確定の文字起こしが Chrome なら、途中の字幕も Chrome。発話の区切りを Chrome の確定で決めるので、アプリはマイクを開かない（`chrome::run`）
- Chrome の翻訳は Chrome のページの中で動くので、途中の字幕が Chrome のときだけ選べる。途中の字幕が Chrome でないのにページを開くと、使わない音声認識まで動いて音声を Google に送ってしまう。操作パネルでは、Chrome の翻訳を使っている間は途中の字幕を Chrome から変えさせない（変えると翻訳が OpenAI に直り、黙って料金がかかる）
- 翻訳と用語は 1 つの工程にする。LLM は訳文の用語を参照（`⟦hero:13⟧`）で書き、Chrome の流れは用語を英語の名前に置き換えてから訳す。どちらも用語の判定の結果で訳文の名前を守るので、別のもので作ると名前が崩れるか、片方の答えを待って遅くなる
- ふりがなは別の工程にする。今も翻訳とは別の問い合わせで同時に投げているので、分けても遅くならない
- 有料の工程は会社ごとに鍵が要る（`Engines::uses_openai` と `uses_xai`）。鍵は自分のキーか、その会社に使えるギフトコード（`Keys::access`）。始めるときに足りない鍵を 1 つ選んで `MissingKey { provider }` で止め、操作パネルは足りない鍵を全部出す（`panel.missingKeys`。ギフトコードがあれば足りるとみなす）。ギフトコードが xAI を中継しないと分かったら、始めるときに `GiftUnsupported { provider: xai }` で止める。使わない会社には鍵があっても何も送らない
- 工程の選択肢には各社の単価（`usage.rs` と同じ）を添える。1 時間あたりの目安は工程ごとには測っていないので書かず、モードの目安と「料金」タブの使い道ごとの額で見てもらう
- モードには 1 時間の目安（`hourlyUsd`。3.3）と、毎日 3 時間配信したときの月の目安を出す（`engines.ts` の `monthly`）。1 時間の値段だけでは配信を続けたときの負担が分かりにくいため。月の目安は円も出し、1 ドル 150 円（`YEN_PER_USD`）で計算する（為替で動くので目安。LP にも書く）

**確定の仕上げ（`finish.rs`）:** 確定の文を Chrome と OpenAI のどちらが作っても、`Finisher` が工程ごとに用語・ふりがな・翻訳を付ける。PC の中の読みと用語（`Annotator`）を先に付け、LLM に聞く工程（`enrich.rs`）と Chrome の翻訳（`chrome::final_translation`。頼んだときから聞くので取りこぼさない）を同時に待ち、聞かなかった工程は PC の中で付けたものを使って、文字の位置で 1 つの字幕にまとめる（`enrich::source_tokens`）。どちらの経路でも、結果はチャンネルで字幕の処理に返し、Segment を書き換えるのは字幕の処理だけにする。LLM に鍵やギフトコードが断られたら（`share::refusal`）、確定の文字起こしと同じく字幕を止める（どちらの経路でも）。Chrome の確定の訳は 10 秒待って来なければ訳なしで仕上げる（ページが固まっても字幕を残さない）。

**起動:** `transcriptionEnabled` がオンで途中の字幕が Chrome なら、アプリが Chrome を起動し、止めたら閉じる（`Store::sync_chrome`。字幕の処理とは別に持つので、設定を変えて字幕の処理を作り直してもウィンドウは閉じない）。全画面のゲームに隠れると Chrome が処理を絞ることがあるので、省電力の働きを切るフラグを付ける。フラグは起動時にしか効かないので、専用のプロファイル（`chrome-profile`）を使う。`--app` で、タブのない小さなウィンドウにする。Chrome を起動できなければ、操作パネルに「Google Chrome が必要です」とインストールのページへのボタンを出す（`LiveState::Missing`）。字幕の途中で閉じられたら「Chrome が閉じられました」と出し、「Chrome を開き直す」で戻せる。閉じられたら、字幕を始め直すか「Chrome を開き直す」を押すまで起動しない（`Browser` を残し、プロセスが終わっているかは見ない）。設定が変わるたびに `sync_chrome` が呼ばれるので、OBS の見た目を変えただけでウィンドウが開いてフォーカスを奪わないようにする。マイクや言語の変更でも起動しない。ページは開いたときに設定を読むので、開き直せば反映される。スタンダードと Chrome の間のモードの切り替えでも起動しない（閉じられたという表示と開き直すボタンが残る）。OpenAI のモードを通るとウィンドウは要らなくなるので忘れ、次に Chrome を使うモードへ切り替えると起動する。Mac ではウィンドウを閉じてもプロセスが残る（Chrome 156 の実測）ので、プロセスの状態では閉じられたかを判断できない。OpenAI を使う工程があって字幕が止まったら Chrome も閉じる（確定の字幕にならない音声を Google に送らない）。Chrome を開いておくかは `Store::chrome_needed` だけで決める（字幕がオンで、途中の字幕が Chrome で、鍵が要るのに字幕が止まっていない）。鍵がないときも `MissingKey` で止まっているので、別の条件は要らない。

**マイクの許可:** 初回に Chrome の確認で利用者が「サイトへのアクセス時は許可」を押す（「今回のみ許可」は次回また聞かれる）。許可は専用のプロファイルに保存されるので、次回からは聞かれない。権限がないと `enumerateDevices` がマイクの名前と ID を隠すので、ページは一覧を読む前に `getUserMedia` で確認を出し、答えを待つ間は「許可してください」と出す。ブロックしたら確認は出なくなるので、操作パネルの「Chrome をリセット」か OS の設定を案内する。

- リセット（`POST /api/chrome/reset`、`Store::reset_chrome`）は、Chrome を止めてプロセスの終了を待ち、専用のプロファイルを丸ごと消して、字幕が動いていれば起動し直す。プロファイルがなくても成功する。プロファイルに入っているものはすべて消えるので、翻訳のモデルもダウンロードし直すことがある。マイクの許可だけを `Preferences` から消す案は、Chrome の内部形式に依存するので採らない。操作パネルでは Chrome の問題（止まった、待っている）を出すときはその中に置く。それ以外は、何のためのボタンか分かるよう「マイクの許可をやり直したいとき、字幕が出ないときに」と添えて小さく置く
- `Browser` の `Drop` が待てるのは本体だけで、子プロセスは遅れて終わる。Windows ではそれまでファイルを消せないはずなので、最大 5 秒試し直し、消せなければ操作パネルにエラーを出す。消している間は `browser` のロックを持ち、`sync_chrome` が同じプロファイルで起動しないようにする
- ボタンは「マイクと翻訳」の Chrome の欄に、Chrome を使うモードならいつも出す。`failed` はマイク以外の理由でも出て、字幕を止めると状態も消えるので、状態では出し分けない

- CDP で許可しない。`--auto-accept-camera-and-microphone-capture` は権限として残らず、`Browser.setPermission` の許可は CDP の接続が切れると消える。さらに、ページは許可より先に動き始めるので読み直しが要るが、最初のページの読み込み中に `Page.navigate` が当たると renderer が `STATUS_ACCESS_VIOLATION` で落ちた（Windows の Chrome 158。起動から 226〜236 ms の送信で落ち、246 ms 以降は 16 回落ちなかった）

**自動化:** 翻訳モデルのダウンロードにはユーザーの操作が要るので、開発者ツールの仕組み（CDP。`--remote-debugging-port=0`）の `userGesture` で満たす。翻訳先を増やすとまた要るので、Chrome が閉じるまで 1 秒ごとに確かめる。失敗したときのために、ページに手で押せるボタンも置く。

- 閉じられたと判断するのは、ページのタブの `Target.targetDestroyed` か CDP の接続が切れたときだけ（`drive`）。`Runtime.evaluate` の応答は読まない。Chrome 156（Mac）の実測では、ページの再読み込みでは失敗しない（1 シナリオ約 550 回）。ページが落ちると応答が来なくなり、そのあと読み込み直すと、待っていた応答に `-32000 Target crashed` が返る。このときプロセス、タブ、セッションはどれも生きていて、同じセッションで動き続ける。ページが固まったままウィンドウを閉じると応答はそのまま来ず、Mac ではウィンドウを閉じてもプロセスと接続が残る。つまり、応答を待つ形では、落ちたページで見張りが止まり、閉じたことにも気づけず、読み込み直すと閉じたと誤る
- ページが落ちただけのときは閉じたと扱わない。`Target.targetCrashed` を見て操作パネルに案内を出し、見張りは続ける。Chrome のウィンドウの「再読み込み」か「Chrome を開き直す」で戻る

**ページ:** `core` が配る `/chrome` のページが `SpeechRecognition`（`continuous`、`interimResults`、`lang` は話す言語）と `Translator` を動かす。文は WS `/api/chrome/ws` でやりとりする（`ChromeMsg`、`ChromeCommand`）。順番が保たれ、切れたらページが閉じられたと分かるので、WS にする。`/ws` と同じ `local_only` の内側に置く。

- ページは Google Chrome でだけ動く（`userAgentData` のブランドで見る）。Edge と WebView2 は音声の送り先が違い、認識が失敗する報告もある
- 字幕の処理が文を受け取れる（`live.status`）ときだけ聞く（字幕にならない音声を Google に送らない）。アプリが落ちたらすぐ聞くのをやめ、戻らなければウィンドウも閉じる
- 聞くのは 1 つのページだけにする。core が `/api/chrome/ws` の接続を持ち、新しい接続が古い接続を閉じる（閉じる理由のコード 4001）。Web Locks はオリジンごとなので、`127.0.0.1` と `localhost` の 2 つのページを止められない。古い接続の後始末は、番号を比べて新しい接続の状態を消さない
- ページの状態は `chrome.status` で操作パネルに出す。`null` はページが開いていない。ページが閉じられたら開いている発話を捨て、訳を待っている発話は訳なしで閉じる
- 回線が切れても、マイクを抜いても止まらない（止めるのは `not-allowed`、`service-not-allowed`、`language-not-supported` だけ）。エラーで終わった認識が 3 回続いたら操作パネルに知らせ（`LiveState::Blocked`）、間隔を最大 15 秒まで広げて聞き直し続ける。文が届く、エラーなく終わる、エラーなく 5 秒続く、のどれかで回線が戻ったとみなす
- マイクのトラックを持ち続ける（キャプチャ中のページは Chrome に凍結されない）。聞き直しはタイマーを待たず `onend` の中で行う。Chromium のコードでは隠れていても認識を止めないが、音声認識字幕ちゃんの説明では最小化すると止まるので、最小化しないよう案内する
- 完全に隠す方法は見つからなかった。`--headless=new` は音声の動作が保証されず、`--silent-launch` はページを開かない
- 操作パネルで選んだマイクのトラックを `recognition.start(track)` に渡す。ブラウザのデバイス ID とアプリのマイク名は別のものなので、ラベルを名前で比べて選ぶ（全体の一致を優先し、なければ末尾の括弧を 1 つ除いた一致。Chrome が足す機器の ID や接続の種類の語の表は持たない。部分一致は別のマイクに当たるので使わない。「既定」「通信」の別名はデバイス ID で除く）。自動で見つからなければ、Chrome のウィンドウに一覧の選択欄を出す。選んだマイクはデバイス ID とラベルを `localStorage` に覚え、アプリのマイクが同じ間は次回も使う（「Chrome の既定のマイク」も選択として覚える。読めなければ選択なしとして続ける）。覚えたマイクを開けないか見つからなければアプリのマイクを試し、使えなかったマイクの名前と使っているマイクを出す。選択欄は実際に開いたマイクに揃え、同じマイクを選び直しても開き直す。マイクだけ開き直すときは翻訳の Translator を作り直さない。`start(track)` のない Chrome 135 より古い版では理由を出す。見つからない、開けない、受け付けない、受け付けても黙って結果が来ない環境（Windows 11 の報告がある）では、結果が来ないまま 2 回作り直したら `start()` に戻す。既定に戻すときは選んだマイクを閉じ、理由と OS の既定のマイクを合わせる案内を操作パネルと Chrome のウィンドウに出す。一度でも結果が届いたら戻さない。Chrome のウィンドウから、マイク名、ラベルの一覧、エラー名、`start(track)` の結果の記録をコピーできる（機器の名前が入るので外に送らない）。入力レベルのメーターは Chrome のモードでは出さない
- 始めてから `onstart` まで 8 秒、動き始めてから 90 秒なにも届かなければ、止まったとみなしてインスタンスを作り直す（`continuous` の認識は黙っている間なにも届かないことがあるので長くする。`onend` のあとの聞き直しの待ちは数えない。無音の `no-speech` は終わりの知らせが届くので数えない）。長時間の配信で黙って止まらないため

**文の流れ:** ページが認識した文 → core（`annotate.rs` で読みと用語を付け、用語を英語の名前に置き換える）→ ページ（`Translator` で訳し、訳文に残った英語の名前を用語に戻す）→ core の順に一巡する。用語の判定は core、翻訳は Chrome の中にしかないので、この順になる。途中の訳は最新だけを作り、確定の訳はすべて作る。読みと用語は 1 文 1ms ほどなので、文を受け取った場所で同期で作り、文とトークンを同時に発話へ載せる（`Annotator`）。読みが遅れて届かないので、文とトークンがずれず、確定でふりがなやアイコンが一瞬消えることもない。

- `chrome`（`chrome::run`）: core はマイクを開かず、VAD も通さない。発話の区切りは Chrome の確定で決まる。文字が文の区切りの時間（`minSilenceMs`）増えなければ認識を止めて確定させ、すぐ聞き直す。話し始めてから起動しないのは、Chrome には過去の音声がなく、最初の言葉を落とすため。音声の処理と共有するのは、発話の形、読みと用語、確定の仕上げ（`Finisher`）、オーバーレイへの出力だけ
- `standard`（`subtitles::run`）: 確定の字幕と翻訳は今までどおりアプリのマイクと VAD から OpenAI で作り、Chrome の文は開いている発話（`Stage::Partial`）に載せるだけ。発話が閉じたら番号付きの `cut` を送り、ページは認識を区切ってすぐ聞き直す。ページは文にその番号を付けて送るので、区切られたあとに届いた前の発話の文は捨てる。発話の途中で Chrome が確定した文はつなげる。文の境界はアプリの VAD で決まる。Chrome が動いていなくても、途中の字幕が出ないだけで確定の字幕は出る。OpenAI の確定の文も、工程の選択どおりに `Finisher` が仕上げる
- 話す言語が日本語でなくても Chrome は認識できるので、どちらのモードも使える。形態素解析（Lindera + IPADIC）は日本語だけなので、ほかの言語の文は読みと用語を付けずに流す
- 翻訳先は Translator が対応する言語すべて。対応しない組み合わせは省いて、操作パネルに知らせる。モデルは Chrome が持つ（初回に Chrome がダウンロードし、進み具合を出す）

**途中で変えた判断:** rc.4 までは途中の字幕を PC の中の認識（SenseVoice）で作り、翻訳を Bergamot で作った。無料のモードの精度が競合の Chrome の認識より低いと使ってもらえず、翻訳先も 4 言語に限られ、別プログラムのダウンロードも要った。そこでどちらも Chrome に任せ、補助プログラム（`jimaku-local`）をやめた。形態素解析は core に組み込む。音声が Google に送られることは、操作パネル、プライバシーポリシー、LP に書く。ふつうのタブで開く形も試したが、フラグが効かないので、専用のプロファイルの起動に戻した。

| 用途 | 採用 | 理由 |
|---|---|---|
| ふりがな、用語の判定 | Lindera + IPADIC | 辞書引きなので速く、API も要らない。文脈で決まる読みや造語は誤るので、`standard` の確定の字幕は LLM で読む。辞書はアプリに組み込まれる（無圧縮で約 46MB。Lindera に圧縮の設定はない） |

**速い用語の判定（`terms.rs`）:** デッドロックモードがオンなら、LLM を使わずに字幕の中の用語を探す。規則は 1 つで、**書き方か読みが公式名と同じなら用語にする**。手で保守する表（別名、聞き違いの書き方、除外する言葉）は持たない。辞書から用語の表（`terms::Term`。公式名ごとに、参照、読み、英語名）を作って探す。辞書はあとから届いたり、LLM の読みが増えて作り直されたりするので、変わるたびに作り直す。

1. 書き方の一致: 形態素の区切りに沿って公式名を探す。同じ位置からは長いものを取る
2. 読みの一致: 形態素の読みをつなげたものを、公式名の読みと比べる（呪いの異物 → のろいのいぶつ → 呪いの遺物）。4 文字より短い読みは比べない

読みの長さの下限だけは残す。外すと公式名と違う書き方の言葉が用語になる。「皆」がミナ、「見な」がミナ、「山と」がヤマトになり、訳文の名前も置き換わる（3 文字まで下げても「山と」が残る）。漢字を含む公式名の読みはどれも 4 文字以上なので、下限が外すのは「へいず」のようなかな書きの短い名前だけ。

漢字を含む公式名の読みは、LLM の読みのキャッシュ（8.2）があればそれを、なければ Lindera + IPADIC が名前を読んだ結果を使う。手で保守する読みは持たない。IPADIC は固有の名前を読み誤る（人狼 → ひとおおかみ、殺魂剤 → やたましいざい）が、読みの一致で見つからないだけで、書き方の一致は効く。誤った読みは普通の言葉の読みと重ならない。正しい読みの同音語（奇襲と紀州、近接と緊切）は用語になることがあるが、例外の表は足さない。

あいまいな一致はしない。文脈を見ないので、結果は次のようになる。

- 公式名と同じふつうの言葉も用語になる（ポケット、ラッシュ、セブン、シルバーなど）。アイテムと同じ言葉（コンデンサー、サイレンサーなど）は `chrome` ならカードも出る
- 略称（カクマガ）と、読みが公式名と違う聞き違い（角磨が）は用語にならない。`chrome` ではアイコンが付かない
- カタカナの複合語（ヘイズウルト）は 1 つの形態素になるので、用語にならず、翻訳の前の置き換えも効かない。カーストの中のカースのように、文字の並びだけでは別の言葉と見分けられない
- どちらも `standard` と `openai` では LLM が文脈から判定するので、確定で直る。略称と言い間違いは LLM に任せる（9.1）

見つけた用語は `Token::Entity` にする（原文は話した言葉のまま）。翻訳の前に英語名へ置き換え、訳文に残った英語名も用語にする。同じ名前のスキルとアイテムはアイテムにする。`chrome` のアイテム説明はこの確定の字幕から出る。`standard` のアイテム説明は OpenAI の結果（`enriched`）を待つ。途中の字幕は揺れるので、カードを出さない。

途中の字幕は認識が進むたびに文が変わるので、用語のアイコンは文に合わせて付いたり消えたりする。`GrowingText` は用語の全体が出てからアイコンを付ける。OpenAI の確定の文には、受け取った時点で読みと用語を付けて、文とトークンを同時に入れ替える。`enriched` が来たら置き換える。途中から付いていた用語とふりがなには、`enriched` の演出を重ねない。途中の字幕になく、LLM だけが見つけた用語には、原文でも訳文でも演出を付ける（`tokens` と比べる）。

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
  | { type: "display"; subtitles: SubtitleDisplay; subtitleStyle: SubtitleStyle; rankLanguage: LangCode; rankAnchor: Anchor; rankStyle: RankStyle; itemCards: { holdSeconds: number; language: string; anchor: Anchor } }  // オーバーレイの見せ方（11・12 章）
  | { type: "audio.level"; db: number }                           // 操作パネルのメーター。字幕を動かしている間、0.1 秒ごとのピーク（dBFS）
  | { type: "usage"; usage: UsageView };                          // 操作パネルの料金。使うたびに
```

- 辞書は `GET /api/glossary` で取得し、オーバーレイ側で持つ（Segment には ID しか載らない）。
- サーバーは直近の Segment を（例: 50 件）保持して、snapshot で送る。
- WS クライアントは指数バックオフで再接続し、再接続したら snapshot で状態を置き換える。
- 接続直後と broadcast の取りこぼし（Lagged）の後は、購読しているすべてのトピックの現在の状態（`snapshot`、辞書があれば `glossary.updated`、ランクがあれば `rank.update`、`transcription.status`、`display`、`usage`）。`audio.level` は持たずに流すだけを送る。

## 11. 字幕オーバーレイ

URL: `/overlay/subtitles`（**パラメータはない**。OBS に一度貼れば、あとは貼り直さない）

オーバーレイは**字幕の箱**（推奨 幅 1000・高さ 400）。置く場所は OBS で決める（Deadlock は画面下中央にスキル欄があるので、その上や画面の左右に置く）。字幕は箱の下から積み上げ、箱の幅で折り返す（CSS の `word-break: auto-phrase` で文節の切れ目を選ぶ。ふりがなの付いた文は長い 1 文節に見えることがあるので、収まらないときは途中でも折る。BudouX は入れない）。大きさを変えるときはソースを引き伸ばさず、プロパティの幅と高さを変える（引き伸ばすと文字ごと伸びる）。

見せ方は操作パネルで変え、設定（`subtitleDisplay`）に保存する。変更は変えた項目だけを送り、サーバーが今の値に重ねる（続けて変えたとき、前の変更を古い値で戻さない）。大きさと、操作パネルで選んだ色などは操作パネルの値が優先する（OBS のカスタム CSS で変えるときは `.captions` に `!important` で書く）。大きな文字・多い行数・多い翻訳先を組み合わせたら、箱の高さも上げる（はみ出した上の字幕は切れる）。設定ファイルに読めない項目があれば、その項目だけ既定値にする（ほかの設定は残す）。縁取りの太さは行ごとの文字の大きさに合わせる。変えたら `display`（トピック `display`）で表示中のオーバーレイに届き、その場で切り替わる。URL を手で書き換える運用はしない。

| 項目 | 意味 | 既定値 |
|---|---|---|
| `showSource` | 話した言葉を出すか。オフなら翻訳だけを出す | `true` |
| `furigana` | ふりがなを出すか。出さないときは読みを問い合わせない（切り替える前の字幕には付かない） | `true` |
| `align` | 揃え（`left` / `center` / `right`）。箱を置く位置に合わせる | `center` |
| `fontSize` | 話した言葉の文字の大きさ（px。12〜96） | `40` |
| `translationFontSize` | 翻訳の文字の大きさ（px。12〜96） | `26` |
| `maxLines` | 同時に出す字幕の数（1〜5。0 は制限なし。画面では 1 / 2 / 3 / 制限なし。入りきらない分は箱の上で切れる） | `3` |
| `holdSeconds` | 確定してから消すまでの秒数（2〜30） | `6` |
| `perCharSeconds` | 長い文に足す秒数（1 文字あたり。0〜1） | `0.15` |
| `minReadSeconds` | 翻訳が届いてから、次の字幕に押し出すまでの秒数（1〜30） | `3` |
| `maxWaitSeconds` | 新しい字幕を待たせる上限の秒数（`minReadSeconds` より優先。0 なら待たせない。0〜30） | `6` |

`showSource` をオフにすると、話した言葉を出さず、翻訳だけを届いてから出す。翻訳ができるまで何も出ないが、日本語の字幕が要らない配信者から求められた。見えない途中の字幕とふりがなは動かさないので、料金は半分以下になる（3.3 節）。翻訳先が 0 のときと、翻訳に失敗した発話では、字幕が消えないよう話した言葉を出す。確定の文字起こしに失敗すると、代わりの途中の字幕がないので、その発話は出ない。話した言葉だけにしたいときは翻訳先を 0 にする。訳文は翻訳先（`translateTo`）をすべて出す。出す言語を減らしたいときは翻訳先を減らす（そのぶん費用も減る）。

### 表示ルール

- 映画字幕のような**中央揃え**。下から積み上げ、最新の発話を一番下に出す。partial の行は下端でその場で更新される。
- 表示時間は `settledAt` から数え、`holdSeconds + 文字数 × perCharSeconds` が過ぎたらフェードアウトする。partial / final の間は消さない。
- `maxLines` を超えたら、一番古い行を読み終えていれば（翻訳が届いてから `minReadSeconds`、既定 3 秒）消す。まだなら一番新しい行を待たせ、`maxWaitSeconds`（既定 6 秒。0 なら待たせない）待ったら古い行を押し出す（`lib/subtitle-lines.ts`。毎回計算するだけで状態は持たない。どちらの条件も時間がたつほど成り立つ方にしか変わらないので、消えた行は戻らない。1 行にしても、翻訳が読めないまま次の発話に切り替わらないように。既定の 3 行なら読み終えた行がたいていあるので、話し続けても途中の字幕はすぐ出る）。秒数はどれも決め打ちの値なので、「字幕の表示時間」のダイアログ（「消すまでの時間」の横の「詳しく設定」）で変えられるようにし、動きの説明もそこに書く。押し出すときは上に動かす（`animate:slide`。`lib/slide.ts`）。
  - `maxLines` が 1 のとき、次の字幕に入れ替わるなら古い行はフェードさせずに消す（上に押し上げられながら消えて、2 行に見えるため）。時間切れで消えるときはフェードする
  - `animate:flip` は使わない。flip は大きさの変化も scale で補間するので、partial → final で行の幅が変わると文字が潰れて見える。
- 文字の変化:
  - partial が伸びるときは、届いた文字を数文字ずつ順に出す（`GrowingText`）。認識の結果はまとめて届くので、そのまま出すとカクカクする。文字を動かす torph は、更新が速くなると新しい文字が別の行に重なって見えたのでやめた
  - OBS のブラウザソースは開いたままなので、接続し直したときに配信中のスクリプトと違えば読み直す（`ws.ts`）。アプリを更新しても古いオーバーレイが動き続けないように
  - enriched になったら文字はそのままで、ふりがな・アイコンをフェードインし、用語の色を白から変える（CSS のアニメーション）。torph はプレーンなテキストしか扱えないので、ルビやアイコンの入る enriched には使わない。訳文は下からふわっと出す
  - 確定前の灰色から白へは色だけ切り替える
- `discarded` になったらすぐに消す。

### 1 発話の中身

```
   [アイコン]インファーナスのultに気(き)をつけて      ← 原文（大きく。ふりがなは <ruby> で上に出す）
   Watch out for [アイコン]Infernus's ult             ← 訳文（小さく。翻訳先の分だけ並べる）
```

- エンティティはアイコンを付けてハイライトする（色は種類ごと: hero / item / ability）。原文では話した言葉のまま（拡マガ、ウルト、一番スキル）、訳文では公式名を出す。話した言葉を公式名に置き換えると、「ヘイズの一番スキルって何だっけ」が「ヘイズのスリープダガーって何だっけ」になり、字幕として話と食い違う
- 原文の用語のふりがなは、公式名どおりに話していれば辞書の読み（名前ごとに確かめたもの。8.2）、略称などなら文の読み（9.2）
- partial は灰色、final は通常の色（翻訳待ち）、enriched で完成形になる。

### 11.1 文字の見た目

配信者が字幕とランクの文字の見た目を操作パネルで調整する。

```ts
interface TextLook {
  color: string | null;   // 通常の文字の色（#rrggbb）
  outline: { width: number | null; color: string | null };  // 幅は文字の大きさとの比（0〜0.5）
  shadow: { enabled: boolean | null; blur: number | null; color: string | null };  // ぼかしは 0〜1
}
interface FontStyle {
  font: string | null;
  weight: number | null;         // 1〜1000（CSS の font-weight と同じ。選択肢は faces.css の面から出す）
  letterSpacing: number | null;  // 字間。文字の大きさとの比（-0.1〜0.5。ランクは -0.1〜0.1）
}
type SubtitleStyle = TextLook & {
  background: { enabled: boolean; color: string | null; opacity: number | null };  // 行の後ろの半透明の帯。濃さは 0〜1
  lineHeight: number | null;  // 行間は 1〜3
  languages: Record<LangCode, FontStyle>;
};
type RankStyle = TextLook & FontStyle;  // 帯はない
```

- 字幕とランクで共通の項目が `TextLook`、言語ごと（フォントに付く）項目が `FontStyle`。帯は字幕だけが持つ。話した言葉と翻訳で別なのは大きさだけで、これは `subtitleDisplay` のまま。言語ごとに色なども変えるときは、`languages` の値に項目を足す
- `null` は標準。オーバーレイは値を受けても CSS を書かず、`Captions.svelte` と `RankBadge.svelte` の既定の見た目を使う。OBS のカスタム CSS で `:root` の `--text-color` などを上書きしている人は、操作パネルで色を選ぶまでそのまま効く。既定の見た目は 0.3 とピクセルまで同じ。LP はこれらを渡さないので既定の見た目になる
- 標準の値は `text-style.ts` にも持つ。操作パネルが今の値として出すためで、描くのは CSS
- 字幕に帯を敷くと、標準では縁取りを 0.08 に細くし、影を消す（`text-style.ts`）。縁取りの幅と影を選んでいれば、そちらを優先する。標準では敷かない
- ランクには帯を敷かない。アイコンの画像は透明な縁を含み、その幅がティアごとに違うので、帯の左右の余白がそろわない。`RankStyle` に帯の項目を持たせず、表せないようにする。v0.4.0 の rc で書いた `rankStyle.background` は、知らない項目として読み捨てる（ほかの項目は残る）
- 変更は JSON Merge Patch で送る（`SettingsPatch.subtitleStyle` / `rankStyle`）。書いた項目だけを変え、`null` で標準に戻す。今の値に重ねてから型と範囲を確かめるので、読めない値はエラーになり何も変えない。全部が標準になった言語はキーごと消す
- 通常の文字の色を変えても、ヒーロー名、スキル名、アイテム名の色分けは変わらない（`--hero-color` などのまま）。ただし文字が縁取りより暗い（相対輝度で比べる）ときは、明るい用語の色が縁取りに紛れるので、OKLCH の色相と彩度を残して明るさを反転した色にする。反転は、白と黒へのコントラスト比（WCAG）が入れ替わる明るさへ移す（`1 - L` では黒に近づきすぎて色味が残らない）。sRGB に収まらなければ彩度を落とす。このときは操作パネルの値として `.captions` に書くので、色分けを変えるなら OBS のカスタム CSS で `.captions` に `!important` で書く
- 確定前の話した言葉は、文字の色を縁取りの色へ寄せて落とす（`--pending-color`。標準の白と黒で #c8c8c8 になる比。透明にすると、太い字の中で後ろの映像が透ける）
- OBS 30 のブラウザソース（Chromium 103）は `color-mix()` や `oklch()` を読めないので、色の計算は `color.ts` で行い、結果を CSS 変数に入れる
- 縁取りと影はどちらも `-webkit-text-stroke` と `text-shadow` で両立する。幅とぼかしは em の値を CSS 変数で渡し、`p` で解決するので、行ごとの文字の大きさに合う。ランクの縁取りは標準も選んだ値も根の要素の文字の大きさ（16px）で決まり、`--scale` に比例しない。ランクの影は標準も選んだ値も 16px に `--scale` を掛ける
- 太さと字間は `p` に inline で書き、カスタム CSS より操作パネルの値を優先する。選べる太さは `faces.css` の `font-weight` から読む（可変フォントは範囲の中の 100 刻み）。面のない太さは、ブラウザが最も近い面に置き換える。太字を合成するのは 600 以上を選び、そのフォントに 600 以上の面がないときだけで、そのときだけ「合成」と出す。読めなかった太さは覚えず、選択肢を開いたときに読み直す。標準のフォントは OS で違うので 400、500、700、800、900 を並べる。フォントを選び直しても太さは残す
- 太い縁取りや大きな影が箱の端で切れないよう、選んだ縁取りとぼかしの幅が余白（左右 12px、下 8px）を超えた分だけ行を内側に寄せる。ランクは右の余白（8px）を同じように広げる（左はアイコンがあり、上下はアイコンの高さの中に文字が収まる）。標準のときは寄せない
- ランクの字間を 10% までにするのは、標準のフォントで長い名前（Ascendant III）がアイコン付きで 400×112 に収まる上限だから（Georgia の 900 のように幅の広いフォントは 5% まで。収まらなければ名前が折り返して下が切れるので、ソースの幅を広げる）。縁取りと影は文字の大きさを変えないので、箱の大きさに影響しない
- 画面は字幕タブとランクタブの「文字の見た目」（`TextStyleFields.svelte`）。フォント（太さを含む）と、字幕では帯のスイッチだけを見せ、文字の色、縁取りの幅と色、影、字間、字幕の行間と帯の色と濃さ、標準に戻すボタンを「詳しく」に畳む。字幕だけの行（帯、行間、言語ごとの字間）は字幕タブ（`DisplayTab.svelte`）から snippet で渡す。「詳しく」の見出しには、中で標準から変えている項目の数を出す（例: 「詳しく（2 つ変更中）」。0 なら出さない。`changedDetails`）。効かない隠れた項目（影を消したときのぼかしと色、帯がないときの帯の色と濃さ）と、出していない言語の字間は数えない。色は `<input type="color">` で、動かしている間も、応答を待ちながら最後の色だけを送る。「標準に戻す」も同じ列で送り、待っている色を取り消す。数は選択肢と「数値で指定」のダイアログ（`StyleNumber.svelte`）
- プレビューは字幕がなくても見本を出す。字幕は `/overlay/subtitles.html?preview=ja,en` で言語ごとの見本、ランクは `/overlay/rank.html?preview` でアイコン付きの見本のランク（アイコンは 12 章）。OBS のソースには `preview` を付けないので出ない

## 12. ランクオーバーレイ（`rank.rs`, `/overlay/rank`）

URL: `/overlay/rank`（パラメータはない。ブラウザソースは出すものに合わせる（名前やポイントも出すなら 400×112、アイコンだけなら 112×112。`rankSize`。アイコンだけなのに箱が横に広いと OBS で置きにくいため）。大きさは下の「大きさ」）。ランク名の言語は操作パネルで選ぶ（`rankLanguage`。話す言語・英語・翻訳先から。既定は話す言語）。名前がない言語は英語で出す。ランクは箱の中の操作パネルで選んだ位置（3×3。既定は左上）に寄せる（`rankAnchor`）

表示は **アイコン（サブランク付き）・ランク名（サブランク付き。例: エミッサリー III）・今のサブランクの中での進み具合のゲージ**。出すものは操作パネルで選ぶ（`rankContent`: ゲージ（`gauge`。既定）/ 名前だけ / アイコンだけ）。サブランクはゲームと同じローマ数字（I〜VI）。

- 文字の見た目は字幕とは別に決める（`rankStyle`。11.1）。フォントは 1 つで、ランク名の言語によらず全部の文字に当てる（6.3）
- 標準の色は字幕と同じく白に黒の縁取り（ゲージの塗りも白）。deadlock-api にはティアの色があるが、ゲームの中では見かけないので使わない
- エターナス（最上位）は上のティアがないのでゲージを出さない。リーダーボードの地域（`rankRegion`）を選んでいて、載っていれば順位（日本語は「アジア 123位」、ほかは「#123 Asia」）、なければ累積のランクポイント（例: 72,000 RP）を出す
- エターナス以外でゲージを計算できないとき（点数がそのサブランクの開始より小さいなど）は、ランク名だけを出す。累積のポイントはエターナスの代わりの表示なので、ほかのランクには出さない
- 順位は `GET /v1/leaderboard/{region}`（Europe / Asia / NAmerica / SAmerica / Oceania。Valve の更新は 1 時間ごと）。各行のアカウントは表示名から推定した候補（`possible_account_ids`）なので、候補が自分の ID だけの行を使う（候補が何人もいる行は同じ名前の別人かもしれないので使わず、ポイントを出す）。リーダーボードは取れてから 15 分経ったら取り直す（取れなければ次の回にまた取りに行き、それまでは前の順位のまま）。2 時間より古い順位は出さない。順位はランク名と同じ言語で書く（ランク名が英語にフォールバックしたら順位も英語の形）

| 用途 | エンドポイント |
|---|---|
| 現在のランク | `GET /v1/players/{account_id}/rank`（最後のランクマッチが終わった時点。`badge` = `tier * 10 + subrank`） |
| ランクポイント | 同じ応答の `last_match.player_rank_final_flat_progress`（累積。サブランク 1 つが 1000、6 番目だけ 1500 なので 1 ティアは 6500（2026-11-05 のシーズン 2 から。前のシーズンは 6 番目が 2000）。ティア t・サブランク s の始まりは `(t-1)*6500 + (s-1)*1000`。7-3 の始まりが 44,000） |
| ランク名 | `GET /v1/assets/ranks?language=…`（設定の言語の分だけ取る） |
| アイコン | `GET /v1/assets/ranks/{tier}/{subrank}/image?format=webp`（サブランクのないティアは `large_webp`） |

- アカウントは Steam の 32bit アカウント ID（SteamID64 からは `76561197960265728` を引く。どちらでも受け付ける）。操作パネルで設定する
- `rank_poll_seconds`（既定 180 秒）ごとに取りに行き、変わったら `rank.update`（トピック `rank`）を流す。接続直後にも送る
- データの取り込みに時間がかかるので、試合が終わってすぐには反映されない（deadlock-api の bot とフレンドになるか、Patreon の優先取り込みで速くなる）
- アイコン（約 75KB）は**バッジと元の画像の URL ごとに** `{データディレクトリ}/rank/{badge}-{URL の SHA-256 の先頭 16 桁}.webp` に保存し（一時ファイルから置き換える）、同じ名前の `/assets/rank/…` で配信する。名前が同じなら中身も同じなので `immutable` で返す。WebP（`RIFF` の `WEBP`）であることを確かめてから保存し、エラーのページなどは保存しない。同じ名前のものがあれば取りに行かない。新しく保存したら、そのバッジの前のアイコン（元の URL が変わる前のものと、以前の `{badge}.webp`）は消す。取れていない間は `image` を出さず、次の回に取り直す。オーバーレイはその間、アイコンと同じ大きさの空の枠だけを出す（届いたときに文字がずれない）
- 操作パネルのプレビューの見本（エミッサリー III の 640 / 1000）は、`GET /api/rank/sample?lang=…` が `RankState` を返し、オーバーレイはそれを描くだけ。本物と同じく、ティア・サブランク・累積ポイントから `state()` で作る。名前は英語（名前のない言語は英語で出す）とランクの言語の分だけティアの一覧から取り、取れた言語は覚えて取り直さない（返す `names` にはそれまでに取れた言語の分も入る）。取りに行っている間は覚えたものをロックしないので、言語を続けて変えても前の要求を待たない。同じ言語の名前やアイコンを同時に求められたら、取りに行っているものを待つ。取りに行ったものは、要求が取り消されても最後まで取って覚える。オーバーレイは開いたらすぐ英語で取り始め、`display` で届くランクの言語の名前かアイコンがなければ、その言語で取る（前の要求は取り消す。取れなかったら、次に `display` が届いたときに取り直す）。届くまでは前の言語の名前のまま出す。本物のランクが届いていれば取らない。進み具合は `progress()`、アイコンは上と同じく取ってきて保存する。ゲームの画像なのでアプリにもリポジトリにも同梱しない。取れなかった名前とアイコンは欠けたまま返して次に開いたときに取り直し、アイコンがなければ（ネットにつながらないなど）灰色の盾の仮の形を出す
- ランク名は言語ごとに（まだ取れていない言語を並べて）取り、失敗した言語だけ次の回に取り直す。外部 API の呼び出しには既定のタイムアウト（接続 10 秒、全体 60 秒）を付け、応答が返らずに止まらないようにする
- アカウント ID を読み取れなければ警告してランクを止める
- 並びは 1 行目にランク名、2 行目に細いゲージ。ゲージに数字は出さず、幅をランク名に合わせる（最低 120px。小さく置いても潰れない）。OBS のカスタム CSS では `--gauge-thin-width` と `--gauge-thin-height` で変えられる。アイコンだけ以外もアイコンは出す
- ゲージのないエターナスは、順位か累積のポイントを出す。ポイントが変わったら torph で桁ごとに数字を動かす
- ゲージに数字（`650 / 1,000`）を重ねる表示（`full`）はやめた。古い設定の `full` は読めない値として既定（ゲージ）に戻り、ほかの設定は残る（`keep_readable`）
- バッジが変わると拡大しながら入れ替わり、ゲージは伸び縮みする
- 当日の勝敗などは今は出さない（必要になったら `match-history` を使う）

### 12.5 アイテム説明オーバーレイ（`/overlay/items`）

字幕でアイテム名が出たら、そのアイテムの説明をゲーム内のツールチップと同じカードで出す。ブラウザソースは幅 420・高さ 480（カードは幅 450 で、ソースの幅に合わせて縮め、箱の中の操作パネルで選んだ位置（3×3。既定は右下。カードの高さはアイテムごとに違うので、下に揃えると下端が動かない）に寄せて出る。`itemCards.anchor`）。

**大きさ**（ランクとアイテム説明）: ブラウザソースの大きさに合わせて描き直す（`lib/overlay-size.ts` の `fitSource`）。既定の大きさ（操作パネルの案内と同じ定数）との比は、幅と高さの小さい方にする（片方だけ変えても切れない）。OBS でソースを引き伸ばすと描いた絵を拡大するのでぼやけるが、プロパティの幅と高さを変えれば描き直すのでくっきりする。大きさの設定は操作パネルに増やさない（OBS でいつもやる操作で済む）。
- ランクは比を `--scale` にして、各部の px に掛ける。ページを `zoom` すると、ポイントの数字のアニメーション（torph）が位置を測って動かすときにずれるため
- アイテム説明のカード（deadlock-ui）は中で大きさが決まっているので、カードを `zoom` する（アニメーションはない）。既定のソースではカード（幅 450）をソースの幅 420 に合わせる（比 × 420 / 450）。ページごと `zoom` しないのは、寄せる位置を決める箱（ソースいっぱい）まで大きくなってしまうため
- 字幕は幅で折り返すので、大きさに合わせて大きくしない（文字の大きさは操作パネルで決める）

- カードは [deadlock-ui](https://github.com/deadlock-api/deadlock-ui)（`@deadlock-api/ui-core`。Stencil 製の Web Components、MIT）の `dl-item-tooltip` を使う。Web Components なので Svelte からそのまま使える。使う部品だけを `dist/components/*.js` から読み込む。開発中のライブラリなのでバージョンは固定する
- 説明のデータは部品が deadlock-api から取る（こちらでは持たない）。言語は `<dl-provider language>` で渡す
- 翻訳まで済んだ字幕（`enriched`）の用語から `kind = item` のものを話した順に拾い、辞書の `className` でカードを出す。1 枚ずつ `holdSeconds`（既定 6 秒）出し、続けて話したものは順番に出す
- 出しているアイテムをもう一度話したら、残りの時間を最初に戻す。待っているアイテムなら何もしない。待ちは 3 枚までにし、それより古いものは捨てる（話から大きく遅れない）
- 接続した時点で説明まで済んでいた字幕では出さない（つなぎ直すたびに出し直さない）。途中の字幕は説明が付いたときに拾う。辞書を読む前に届いた字幕は、読めてから拾う
- 操作パネルのプレビュー（`/overlay/items.html?preview`）では、アイテム名を話していなくても見本のカード（基礎拡張マガジン）を出しておく（位置や大きさをすぐ確かめられる。テスト字幕を流して待たなくてよい）。OBS のソースには `preview` を付けないので出ない
- 設定（`itemCards`）: 出しておく時間（3〜30 秒。読み込んだ値が範囲外なら既定値）、説明の言語（既定は話す言語）、カードの位置（`anchor`）。`display` メッセージで deadlock-api の言語名（例: `japanese`）にして届ける。deadlock-ui が対応していない言語は英語で出す

## 13. 操作パネル（control.html）

Tauri のウィンドウでも OBS のカスタムブラウザドックでも同じ画面（`http://127.0.0.1:4510/`）。shadcn-svelte（Tailwind CSS v4）で作る。オーバーレイは軽さと OBS のカスタム CSS での上書きやすさを優先し、Tailwind を入れずに素の CSS のまま。

- **上部に固定**: 状態（停止中 / 準備しています / 字幕を出しています / 音が届いていません / 止まっています）、起動してからの料金（押すと「料金」タブ。無料の Chrome のモードでは出さない）、開始・停止のボタン（選んだモードに要る鍵がなければ、代わりに「キーかギフトコードを入れる」と「無料の Chrome モードで始める」）。アプリの名前はウィンドウ（OBS ならドックのタブ）に出ているので置かない
- **はじめかた**: 中身の欄の一番上にチェックリストを出す（OBS に字幕を追加 → 字幕を始める → 字幕が出る。鍵の要るモードを選んだ人にだけ、先頭に API キーかギフトコードの手順を足す）。全画面の案内にしないのは、途中で OpenAI のサイトへ移る流れと相性が悪く、閉じると見返せないため。チェックは今の状態から付けるので「完了」の操作はない。OBS は、字幕オーバーレイが `window.obsstudio` を見て `obsSeen` を書き（操作パネルのプレビューは iframe なので除く）、初めて書かれたら `settings.changed` を流して操作パネルに知らせる。最後の手順は音量ではなく字幕が実際に出たかで見る（無効なキーでも音は届くし、雑音でも音量は上がるため）。全部終えるか閉じたら `onboardingClosed` を書いて、次からは出さない
- **版と About**: サイドバーの下（狭いときはページの一番下）にアプリの版を小さく出す（不具合の報告と、更新できたかの確認のため）。押すとダイアログを開き、作者、使い方、Discord、利用規約、プライバシーポリシー、ライセンスへのリンクを出す。毎日見るものではないので、サイドバーの項目にしない。OBS のドックのように狭いときは出さない（Tauri のウィンドウで見られる）
- **注意は中身の欄の上に出す**: 選んだモードに要る鍵がない（スタンダードか OpenAI で、API キーもギフトコードもない。「キーかギフトコードを入れる」と「無料の Chrome モードにする」）、字幕が止まった理由、設定を保存できない
- **無料のモードが基本**: 新しく入れた人の既定は Chrome のモードで、使う人の大半はこのまま。モードの選択肢は Chrome を先頭に置き、スタンダードは「高速・高精度」としておすすめの印を付ける（確定まで待つ OpenAI だけのモードより、途中の字幕が速い）。鍵の話は鍵の要るモードを選んだときだけ出す
- **タブの並び**: マイクと翻訳 / オーバーレイ（字幕・アイテム説明・ランク）/ 有料のモード（API キー・おすそ分け・料金）。OBS に置く 3 つは同じ種類なので、名前の形を揃えて「オーバーレイ」の見出しの下にまとめる。鍵の要るモードにしか関わらない 3 つは下のほうの「有料のモード」の見出しにまとめる（狭いときは見出しを出さない）
- **タブは左のサイドバー**: デスクトップのウィンドウ（幅 768px 以上）では、タブをアイコン付きで左のサイドバーに縦に並べ、中身の欄だけをスクロールする。OBS のドックのように狭いときは、上に横に並べる（アイコンは出さず、入りきらなければ横にスクロール）。同じ Tabs 部品の向き（`orientation`）を幅で切り替える
- **オーバーレイのタブは 2 列**: 字幕・アイテム説明・ランクは `OverlayPage.svelte` で並べる。サイドバーがあって中身の欄が 44rem 以上（コンテナクエリ）なら左右半分ずつにし、左に設定、右にプレビューと OBS に追加（`AddToObs.svelte`）を止めておく（設定を変えながら見た目を確かめられる）。狭いときは 1 列にしてプレビューを上に置く。プレビューの下に OBS のソースの大きさを書く。OBS に追加は小さくまとめ、長い説明と URL は「うまくいかないとき」の窓に入れる（ドラッグする部分は窓の外）。ほかのタブは読みやすい幅に留める
- **URL を手で書き換えさせない**: 見せ方はすべて画面の操作で変え、OBS に貼る URL はコピーするだけ
- **OBS へはドラッグで追加**: 「OBS に追加する」のボタンを OBS のプレビューかソースの一覧にドラッグすると、名前と大きさを合わせたブラウザソースができる（URL に `layer-name` / `layer-width` / `layer-height` を付ける。OBS 25 以降。OBS を管理者として起動していると使えない）。うまくいかないときのために URL のコピーも残す。名前に空白は使わない（`+` になり、OBS がそのまま読むことがある）。ドラッグする部品はリンクにせず、渡すのは URL だけ（ブラウザが付けるショートカットのファイルなどを OBS が先に読むと、ソースができない）。Tauri の画面はファイルを落とす処理を切る（Windows ではページの中のドラッグが働かなくなる）

| タブ | 内容 |
|---|---|
| マイクと翻訳 | 入力デバイス（一覧の更新つき。見つからなければそう出す）、入力レベルのメーター（-60〜0 dB）、5 秒間 -60 dB を超えなければ「音が届いていません」、話す言語、話している途中から字幕を出すか、デッドロックモード（オフにすると用語ヒント・用語の判定・アイテム説明を止める。雑談の配信向け。用語ヒントはセッションの開始時にしか渡せないので、切り替えると字幕を始め直す）、翻訳先（最大 4。OpenAI のキーがなければ注意）、文の区切り（すぐ 0.5 秒 / ふつう 0.75 秒 / つなぐ 1 秒。ミリ秒は「数値で指定」のダイアログ） |
| 字幕 | プレビュー（オーバーレイを推奨サイズ 1000×400 で開いて縮めたもの）とテスト字幕、11 章の見せ方（揃え、文字の大きさは話した言葉と翻訳を別々に、px の数値の選択肢で（話した言葉 16 / 24 / 32 / 40、翻訳 16 / 20 / 26。既定が一番大きい。もっと大きくするなら「数値で指定」）、大きさは「数値で指定」でも決められる）、「文字の見た目」の節（11.1）に話す言語と翻訳先の行ごとのフォントと太さ（6.3。ダイアログで検索してカードから選ぶ。先頭は「標準」で、Google Fonts、この PC のフォントの順に並べる。この PC のフォントは一度に 500 件まで描く）と半透明の帯。「詳しく」に文字の色、縁取り、影、行間、字間、帯の色と濃さ。文字の色で変わるのは通常の文字だけで、ヒーロー名はピンク、スキル名は水色、アイテム名はゲーム内のスロットの色のまま（縁取りより暗い文字では、同じ色味の濃い色）。この色分けを変えたい人は OBS のカスタム CSS で `--hero-color`・`--ability-color` を上書きする。並べた選択肢で足りない値は、どの設定も見出しの横のボタンから開くダイアログで決める（変えた項目だけを送る）、OBS に貼る URL と置き方 |
| アイテム説明 | プレビュー（420×480。見本のカードを出しておく）、出しておく時間（4 / 6 / 8 / 12 秒か「数値で指定」）、説明の言語、位置（3×3）、OBS に貼る URL |
| ランク | プレビュー、位置（3×3）、Steam のフレンドコード（「フレンドを追加」に出ている番号。SteamID64 も可。空で非表示）、ランク名の言語、「文字の見た目」の節（11.1。行間と帯はない）、OBS に貼る URL |
| API キー | ギフトコード（人からもらったおすそ分けのコード。17.1。入れたら使えるか確かめてから保存し、期限と、Grok（xAI）にも使えるかを出す。自分のキーがあればその会社には自分のキーを使う）。OpenAI の API キーの作り方（登録 → クレジットカードの登録と前払いのチャージ → キーの作成 → 貼り付け。OpenAI を使ったことのない配信者向け。キーが保存されていなければ開いておく）。OpenAI（文字起こし・翻訳・読み）と xAI（Grok の確定の文字起こし）。状態（保存済み・未設定・読めない。読めないときは理由を出す）、キーの作成ページへのリンク、新しいキーの保存・削除。キーそのものは表示しない |
| おすそ分け | ギフトコードを作る（メモ・期限）、作ったコードの一覧（伏せ字のコードとコピー・表示、期限、今月と合計の額、期限を変える、止める・戻す）、預けるキーを会社ごとに選ぶ（OpenAI と xAI を同じ形で並べ、確認してから預ける。自分のキーがあるときだけ預けられる。最後の 1 つはやめられない）、ほかの PC のおすそ分けを表示する、おすそ分けをやめる（預けたキーとギフトコードを消す）。17.1 |
| 料金 | OpenAI と xAI の料金の概算を、使い道（途中の字幕・確定した字幕・用語と翻訳・ふりがな・辞書の読み）ごとに「起動してから」と「今月（UTC）」で出す。おすそ分けしていれば、その分（今月）も足す（同じキーに請求されるので）。ギフトコードで使っているときは、どの会社の分がおすそ分けした人に請求されるかを添える（`panel.giftPaid`）。正確な額は OpenAI と xAI の使用量のページへ |

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

- `/api` と `/ws` は Host ヘッダーが `127.0.0.1` / `localhost` / `[::1]` のときだけ受け付ける（DNS リバインディング対策）。Origin があれば、呼ばれたアドレスと同じもの（`http://{Host}`）だけを通す（別のサイトからの呼び出しの対策。本文を読まない `POST /api/share` や `POST /api/demo` と WebSocket はプリフライトなしで届くので、JSON に限るだけでは防げない）。`Origin: null` も通さない。no-cors の GET（`<img src>` など）には Origin が付かないので、`Sec-Fetch-Site: cross-site` も断る（`same-site` は Origin の判定に任せる）。どちらも付かないもの（curl など）は通す。ポートの値は持たない。Vite の開発時は、プロキシが Host も Origin も `localhost:5173` のまま渡すので同じ形になる（文字列の proxy は Host を書き換えるので、`vite.config.ts` の `/api` はオブジェクトの形にする）
- 残り: 辞書の状態、直近の Segment のログ、ミュート（グローバルショートカット）

## 13.6 トレイ常駐（`app/src/tray.rs`）

ウィンドウの閉じるボタンでは終了せず、WebView ごとウィンドウを閉じる（隠すだけだと WebKit のメモリが残る）。開くときに作り直す。設定と字幕はアプリの本体が持ち、ウィンドウの大きさと位置は tauri-plugin-window-state が保つ。字幕とオーバーレイはアプリが配信しているので、誤って閉じると配信中に全部止まるため。

- トレイのアイコンを押すとウィンドウが戻る。メニューは 開く / 字幕を始める・止める / 終了。メニューの文字と説明は設定の `transcriptionEnabled` に合わせて変える
- 初めて閉じたときだけ、動き続けていることと終了の仕方を知らせる（`trayNoticeSeen`）。気づかずに字幕が動き続けると料金がかかるため
- 字幕が止まったら説明の文字で分かるようにする（ウィンドウを閉じていても）。キーがなければ、トレイの「字幕を始める」は操作パネルを開く
- 更新は起動したときに確かめ、そのあとは字幕を止めている間に 6 時間ごとに確かめる。「あとで」を選んだら、次に起動するまで聞かない
- 単一起動にする。閉じているときにもう一度起動すると、ポートを取り合わずにウィンドウを出す
- Mac は Dock のアイコンを押しても戻る
- Linux はトレイの出ない環境があり、閉じると戻せなくなるので、今までどおり閉じたら終了する
- Linux では `tray-icon` を有効にしない（`app/Cargo.toml`）。有効にすると GPL-3 の libayatana-indicator と ayatana-ido が AppImage に入る。Tauri の CLI は `Cargo.toml` の `tauri` の features を OS を区別せずに読み、`tray-icon` があれば deb の依存と AppImage に libayatana-appindicator を足す。`workspace = true` の依存の features は読まないので、`tauri` は workspace の依存にする

## 13.5 自動更新（`app/src/main.rs`、tauri-plugin-updater）

- 起動したときに GitHub の最新のリリースの `latest.json` を読み、新しい版があれば「更新しますか？」のダイアログを出す。「更新する」で入れ替えて再起動する（字幕は一度止まる）。開発中のビルドでは確かめない
- 更新のファイルは署名して配り、アプリは `tauri.conf.json` の公開鍵で確かめる。秘密鍵とそのパスワードは Secrets（`TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）にだけ置き、リリースのビルドの「署名して作る」手順にだけ渡す。秘密鍵をなくすと、今のアプリに更新を配れなくなる
- 更新のファイルはリリースのときだけ作る（`--config '{"bundle":{"createUpdaterArtifacts":true}}'`。PR のビルドに秘密鍵を渡さない）
  - macOS: `deadlock-jimaku_universal.app.tar.gz`（Apple シリコンと Intel の両方入り。`darwin-aarch64` と `darwin-x86_64` に同じものを書く）
  - Windows: NSIS のインストーラー（`windows-x86_64-nsis`）
  - Linux: AppImage だけ（`linux-x86_64-appimage`）。deb・rpm はそれぞれのパッケージの仕組みで入れ直すので、更新の知らせを出さない（`latest.json` に載せない）
- `latest.json` はリリースのワークフローで作る（署名の `.sig` の中身を入れ、`.sig` 自体はリリースに載せない）
- v0.1.2 までのアプリには自動更新がないので、一度だけ手で入れ直す

## 13.7 第三者のライセンス表示（`notices/`、`frontend/scripts/notices.ts`）

- `notices/third-party.txt` は `mise run notices` で作ってコミットする。手で編集しない。IPADIC と Lindera は別のファイルのまま（利用規約でも読む）
- 載せるのは配るものだけ。Rust は `cargo tree --edges normal,no-proc-macro` を build.yml の 4 つのターゲットごとに解決し、リンクされる crate を集める（dev-dependencies、build-dependencies、proc-macro は成果物に入らない）。フロントはほぼ全部が devDependencies なので、`vite build` の結果に実際に残ったモジュール、フォントなどのアセット、CSS の `@import` から取る
- 外の道具（`cargo-about` など）は使わない。フロントの分と NOTICE は結局自分で書くので、Node のスクリプト 1 つのほうが単純で、版の固定も要らない
- 著作権表示はパッケージの中のライセンスファイル（サブフォルダの分も）から取る。同じ本文は 1 度だけ書く。`OR` のものも選り分けず全部のファイルを載せる（本文は重ねないので大きくならず、SPDX の式を読まなくて済む）。ファイルがないものは、推測で埋めず一覧に分けて出す
- crate とバンドルから分からないもの（Rust の標準ライブラリ、WebView2 SDK の静的ライブラリ、NSIS、AppImage のランタイムと同梱ライブラリ）は `notices/platform.txt` に手で書き、生成のときに差し込む。WebView2 SDK や Ubuntu の版を上げたら見直す
- About からは開いたときに読み込む（`import("...?raw")`。全文を初期のバンドルに入れない）
- Google Fonts のフォントはアプリに含まないが、`notices/fonts.txt`（手で管理。ファミリーごとの著作権表示と OFL の本文を 1 度）を About に出す。`font-catalog.json` の全部のファミリーが載っていることをテストで確かめる。見本の SVG は数文字の輪郭で、OFL の及ばないフォントを使って作った文書として扱い、同じファイルに出所を書く。利用規約の「使っているソフトウェア」はアプリに組み込むものの表示なので、含めない
- `mise run check` と build.yml（Linux）が `--check` で作り直して比べ、古ければ失敗する。3 秒ほどで済む

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
| `server/` | `api.jimaku.deadlock.jp` | Hono。パスは版（`/v1`）で区切る。アプリ専用なので `jimaku` の下に置く（`deadlock.jp` 全体の API に見えないように） |

- **リリースのファイル名**: 版の番号を付けない（`deadlock-jimaku_x64-setup.exe`、`deadlock-jimaku_universal.dmg`、`deadlock-jimaku_amd64.AppImage` / `.deb`、`deadlock-jimaku.x86_64.rpm`）。LP は `releases/latest/download/<ファイル名>` で最新版を直接落とすので、名前が変わらないようにする。版はリリースの名前で分かる（v0.1.2 から）
- **名前**: 表示名は「デドロ字幕」（画面・ウィンドウ・インストーラー。Linux だけはパッケージ名に日本語を使えないので、アプリ一覧の名前も `deadlock-jimaku`）。英数字の名前は `deadlock-jimaku`（実行ファイル・インストーラーのファイル名・Linux のパッケージ名。日本語の「デドロ」を知らない人にも Deadlock の道具だと分かり、ドメインとも揃う）。Worker（`deadlock-jimaku-api` / `deadlock-jimaku-web`）とリポジトリ、D1 も同じ名前にする。アプリの識別子は `jp.deadlock.jimaku`（ドメインを逆にしたもの）。データの置き場所と資格情報ストアの名前を兼ねるので、変えると設定とキーは引き継がれない（v0.1.4 で `dev.deadlock-stream-overlays.app` から変えた。開発中で配っていないので、入れ直す）。v0.1.0〜v0.1.1 の英数字の名前は `dedoro-jimaku` だったので、Linux のパッケージはそれを置き換える（deb の replaces / conflicts、rpm の obsoletes）
- **日本語だけ**: ドメインと利用者が日本向けで、ふりがなのように日本語に寄せた機能なので、LP もアプリの画面も日本語だけにする（英語に対応すると、文言を足すたびに両方書く手間が続く）
- **API**: おすそ分け（17.1）。以前は別名を配っていたが、用語の判定を LLM に任せたのでやめた（9.1）。前のドメインは `api.deadlock.jp`
- **Mac の dmg**: 開くと左にアプリ、右に Applications、間に矢印が出るようにする（`app/tauri.conf.json` の `bundle.macOS.dmg`）。背景は `app/dmg/background.html` から `mise run dmg-background` で作る（1 倍と 2 倍をまとめた tiff なので Retina でもくっきりする）。CI では Tauri が見た目を付ける処理を飛ばすので、`TAURI_BUNDLER_DMG_IGNORE_CI` で飛ばさせない。Finder のアイコンの文字は暗い色なので、背景は明るくする
- **API キーの作り方**: LP にも節を置く（`#openai-key`）。キーが要ることと手順を、入れる前に見られるように。文はアプリの案内（`OpenAiKeyGuide.svelte`）と揃える
- **連絡先**: Discord のサーバー（招待リンクは無期限）。GitHub の Issues より配信者に身近で、ほかの人の質問も見られる
- **ダウンロード**: LP の「ダウンロード」の節に OS ごとのボタンを置き、`releases/latest/download/<ファイル名>` で最新版を直接落とす（GitHub のページに飛ばさない）。見ている OS の欄を目立たせる（iPad は Mac と同じ UA を名乗るのでタッチで見分け、Linux は x86_64 だけ）。一番上の「ダウンロード」は節へ移るだけにする（いきなり落とすと、すぐ下の「初めて開くときの警告の進め方」を見ずに止まってしまう）。警告の進め方や AppImage の実行の許可も欄に書く
- **LP**: 明るい紙色の地に、黒い配信画面を置く。一番上で実際のゲーム映像（`web/src/assets/hero.mp4`。字幕などを切って撮ったもの）の上に見本の字幕を流し、言語を押すと翻訳が変わる（`web/src/components/LiveDemo.svelte`。動画に焼き込まず部品を重ねるので、切り替えられ、スマホでも読める。OS の「動きを減らす」では止める）。見本の台詞はアプリの流れ（途中の字幕 → 色・アイコン・ふりがな → 翻訳）をなぞる。字幕とランクはアプリの部品（`frontend/src/lib` の `Captions.svelte`・`RankBadge.svelte`）をそのまま使い、台詞と辞書だけを `web/src/lib/lines.ts` に用意し、`demo.ts` で画像を付けて辞書を渡す（見た目を二重に持たない）。アイテム説明のカードはアプリと同じ deadlock-ui を描く（データはビルドのときに deadlock-api から取って埋める。画像は訪問者のブラウザが取る）。節はほかの字幕アプリとの違いの順に並べる（精度、ふりがな、デッドロックの用語とアイテム説明、ランク、文字の見た目、軽さ）。リアルタイムに出ることは売りにしない。続けて OBS（`obs-drag.mp4`。v0.4 の操作パネルからドラッグして足すところ。止まった 1 枚は字幕が出ているコマから作った `obs-drag.webp`。見えている間だけ流し、「動きを減らす」では流さずこの 1 枚を出す）、料金、はじめかた、ダウンロード（API キーの作り方は開閉式）、よくある質問の順（作者はよくある質問の中）。ほかの人を特定できるもの（名前・試合 ID など）は映さない。フッターに Valve の公式ではないことと、ゲームの画面・データ・アイコンが Valve の著作物で、データとアイコンは deadlock-api から取っていることを書く。プライバシーポリシーには、OpenAI・deadlock-api・開発者のサーバー（おすそ分けのときだけ。何を保存するかは 17.1 の「保存の仕方」と「表」に合わせる）に送るものと、PC の中に保存するものを書く。サイトのアイコン（favicon）は `favicon.svg`（アプリのアイコンと同じ形を手で描いたもの）と `favicon.ico`・`apple-touch-icon.png`（アプリのアイコンから作ったもの）。アプリのアイコンを作り直すときは、元の SVG を 1 つにして両方を作る。
- **LP の静的ファイルのキャッシュ**: 動画・画像・フォントは `web/src/assets/` に置いて `import ... from "...?url"` で読み、Vite が中身のハッシュを名前に付けた `/_astro/*` に出す。`web/public/_headers` で `/_astro/*` を 1 年の `immutable` にする（Workers の静的ファイルの既定は毎回確かめ直す）。中身を変えれば URL が変わるので、同じ名前のまま差し替えてよい。`public/` には URL を変えられないもの（favicon、`og.jpg`）だけを置く
- **LP のモードの書き方**: モードの欄は操作パネル（`AudioTab.svelte`）と同じ一覧（`frontend/src/lib/engines.ts` の `MODES`。並び、名前、特徴・値段・説明、おすすめ）を読み、LP では必要なものだけを足す。根拠を示せない比較（「業界標準」など）は書かない。見本はスタンダードの字幕で、略称の色・アイコンと正式名への翻訳はスタンダードと OpenAI のモードでしか付かない（9.6）ので、見本の下と節の文でどのモードの話かを書く（Chrome のモードで入れた人が食い違いを感じないように）。Chrome のモードは日本語の文にしか用語を付けない（9.6）ので、色とアイコンが付くのは日本語で正式な名前を話したときだけと書く
- **LP の文字の見た目の見本**: `web/src/components/LookDemo.svelte`。`Captions.svelte`・`RankBadge.svelte` に操作パネルと同じ型の値（`TextLook`・`FontStyle` から作る `SubtitleStyle`・`RankStyle`）を渡し、ボタンで見た目を切り替える。見本の値は `web/src/lib/looks.ts` に置き、操作パネルで選べるものだけにする（アプリで同じ見た目を作れる）。フォントはカタログ（`font-catalog.json`）の日本語のフォントとその太さで、字幕の日本語の行とランクに当て、英語の行は標準のまま（日本語だけのフォントを英語に当てない）。フォントはアプリと同じく `lang.css` の `--font` で当たる（LP も `global.css` で `lang.css` を読む）。背景は `hero.webp` の HUD のないところを拡大する。フォントは見本の字（台詞の `web/src/lib/lines.ts` の日本語と ASCII）だけを入れた woff2 を `web/src/assets/fonts/` に置く（Google Fonts の配信を使わず、訪問者の情報を Google に渡さない。ライセンスは woff2 の名前の表に残し、同じ場所の `OFL.txt` にも置く）。`mise run lp-fonts`（`web/scripts/fonts.ts`）がカタログのコミットのファイルを SHA-256 で確かめて作り、`mise run check` の `--check` が、見本のフォントと太さのファイルがあり、見本の字がすべて入っているかを確かめる。台詞や見本のフォントを変えたら作り直す。見本が画面に入って動き始めたら（`client:visible`）3 つのフォントを `FontFace` で読み始め、押したときは読み終えてから切り替える（別のフォントで一瞬出さない）
- **LP の見出しのフォント**: 本文は端末のフォントのまま（字幕の見本とふりがなの位置をアプリと揃える）で、大きい見出し（`.display`）だけ Noto Sans JP の Black（900）を読み込む。見出しは 900 で組むが、Windows の Yu Gothic UI は Bold までしかなく細く出るため。見出しの字だけを入れた woff2（約 45KB）を `web/scripts/fonts.ts` が作る。字はビルドした LP（`dist/*.html`）の `.display` の要素から拾うので、`mise run lp-fonts` は LP をビルドしてから作る。見出しを変えたら作り直す。字が足りないかは `fonts.ts --check` が確かめ、CI（`web.yml`）とデプロイ（`pnpm deploy`）はビルドしたあとに流す（`mise run check` はビルドしないので、dist がなければ見出しの字は確かめない）。最初に見える見出しのため、Layout で preload する。可変フォントの太さを固定すると名前は既定の面のまま（"Noto Sans JP Thin"）になるので、名前は前の方だけ見て、太さは `usWeightClass` で確かめる
- **D1 の表を変えたとき**: `server/src/schema.ts` を直して `pnpm db:generate`（drizzle-kit）で `server/migrations/` に SQL を足し、デプロイの前に `wrangler d1 migrations apply deadlock-jimaku --remote` を実行する（手元は `--local`）。`deploy` のスクリプトは表を変えない
- **リポジトリの旧名**: `Chiji1108/deadlock-stream-overlays` は v0.1.3 の自動更新の参照先として GitHub のリダイレクトに頼っているので、同じ名前のリポジトリを作らない（作るとリダイレクトが消え、v0.1.3 が更新を受け取れなくなる）
- **デプロイ**: 手元から `pnpm --filter ./web deploy` / `pnpm --filter ./server deploy`（先に `wrangler login`）。カスタムドメインは各 `wrangler.jsonc` の `routes`（`custom_domain: true`）で付く
- サーバーは Hono（最初から Cloudflare Workers 向け）。ElysiaJS も考えたが、Workers 対応が実験的なので選ばない
- D1 は Drizzle ORM で読み書きする。表の形（`schema.ts`）から問い合わせの型と migration の SQL を作り、SQL と型がずれないようにする。migration を当てるのは wrangler のまま（Drizzle の migrator は使わない）。0001〜0003 は Drizzle を入れる前に手で書いたので、`migrations/meta/` にはその 3 つを並べた記録と、0003 を当てたあとの形（`0003_snapshot.json`）を置いた。0004 で表をすべて Drizzle の形（索引の名前も）に作り直したので、本番の表と `schema.ts` は同じになった。表を作り直す変更は、作られた SQL を手で直す: D1 は外部キーを常に確かめ、`PRAGMA foreign_keys=OFF` が効かず、表を消すと `ON DELETE CASCADE` で子の行も消えるため。新しい表を別の名前で作って写し、古い表を子から順に消してから名前を戻す（`0004_share_keys.sql`）。drizzle-kit が名前の変更かを対話で聞くときは、作り直す SQL を手で書き、スナップショットだけを drizzle-kit に作らせる

### 17.1 おすそ分け（`server/`）

自分の API キー（OpenAI か xAI、または両方）を持っている配信者が、知り合いに「試しに使ってみて」とコードを渡せるようにする。コードを持つ人の通信は Worker が中継し、おすそ分けした人が預けたその会社のキーで送る。

```
コードを使う人のアプリ ──(コード)──> Worker ──(おすそ分けした人のキー)──> OpenAI / xAI
```

- **預けるキー**: 会社ごとに選んで預ける（`issuer_keys`。会社ごとに 1 つ）。OpenAI のキーがなくても、xAI のキーだけでおすそ分けできる。おすそ分けタブに会社を同じ形で並べ、会社ごとに確認してから預ける（あとから別の会社のキーを黙って送らない）。キーが使えるかは各社のモデル一覧で確かめる（間違ったキーは 400）。コードは会社ごとに分けず、どれも預けたキーの会社に使える（使う会社はもらった人のモードで決まり、額はコードごとの 1 日の上限で抑える。コードごとに分けると、もらった人のモードと合わずに始めるときに断られることが増える）。預けたキーは消せる（替えたときと同じく合言葉も替わる）が、最後の 1 つは消せない（409 `lastKey`。消すとどの PC からもその人に戻れなくなるので、「おすそ分けをやめる」を使う）。自分のキーを替えたら、その会社を預けているときだけ預けたキーも替える（消しても預けたまま）。`GET /v1/gift` は中継する会社（`providers`）を返し、アプリは字幕を始めるときにこれで、選んだモードの会社を中継しないコードを止める（前の Worker は返さないので OpenAI だけとみなす）。動いている間に預けたキーが消されたら、中継が 403 `unsupported` で断り（Realtime は 30 秒ごとの確認で切る）、止まる。料金はどの会社の分も同じコードの 1 日の上限と月の集計に入る
- **API**: `POST /v1/share`（`{ provider, key }` でキーを預けて始める。同じキーを預けた人がいればその人になる。`join: true` なら新しく始めず、預けていないキーは 404。0.4 までの `{ key }` は OpenAI）、`PUT` / `DELETE /v1/share/keys/:provider`（預ける・替える・消す。新しい合言葉を返す）、`PUT /v1/share/key`（0.4 までのアプリ。OpenAI の PUT と同じ）、`GET /v1/share/codes`（コードと `providers`）、`DELETE /v1/share`。rc.4 の `/v1/share/xai` は残さない（プレリリースだけで使った）

- **おすそ分けする人**: おすそ分けタブでキーを預けるときに確認を出し、同意したらキーを Worker に預ける（このときだけ）。最初の 1 つを預けるとおすそ分けが始まり、ギフトコードを作れるようになる。全員のキーを最初から預かることはしない（使わない人のキーまでサーバーに置かない）。Worker は管理用の合言葉を返し、アプリは資格情報ストアに置く（ログインやメールアドレスは使わない）。預けたキーのどれかと同じキーで預け直したら同じ人として扱い、コードは残る。「おすそ分けをやめる」と、預けたキーとコードを消す
- **複数の PC**: 預けたキーのどれかを持つ PC は、同じ人として扱う（「ほかの PC のおすそ分けを表示する」。この PC のキーを OpenAI、xAI の順に `join` で試し、使えないキーは飛ばす。おすそ分けしていない PC でキーを預けるときも、先にこれで探してから足すので、同じ人が別々のおすそ分けにならない）。合言葉は人ごとに決まった値（`HMAC(SHARE_KEY, id と token_salt)`）なので、どの PC でも同じコードと使用額を見て管理できる。預けたキーを替えるか消すと `token_salt` が替わって合言葉も替わり、ほかの PC は登録前の状態に戻る（漏れたキーで得た合言葉を残さない）。0004 より前の合言葉はそのまま使え、どこかの PC がキーを預け直したときに新しい決め方に替わる（ほかの PC は一度「表示する」をし直す）。2 台目の PC はコードを作らずに一覧だけ出すこともできる。コードそのものはハッシュだけを保存するので、見られるのは作った PC だけ。「おすそ分けをやめる」は全部の PC に効く
- **コード**: `K7M2-9QXT-4HNB`（Crockford Base32 の 12 文字、60 ビット）。ランダムなので当てられず、試す回数の制限や人数の上限は置かない（友達に渡すもので、同じコードを何人で使ってもよい。使える額は 1 日の上限で抑える）。手で打つときの区切り・大文字小文字・O と 0・I と L と 1 の違いは揃える。おすそ分けした人だけが見るメモを付けられる。コードはこのときの応答でだけ返し、Worker はハッシュだけを持つ
- **期限**: 終わりの日時を選ぶ（1 日後 / 3 日後 / 1 週間後 / 30 日後のボタンと日時の欄。期限なしも可）。あとから延ばしたり縮めたりでき、「止める」ですぐ使えなくできる（戻すこともできる）。配信の途中で字幕を止めないよう、期限はアプリが動いていない字幕を始めるとき（エラーで止まった状態から、設定やコードを変えて再開するときも）にだけ確かめる（`GET /v1/gift`）。回線の失敗で確かめられなければ始めない。動いている間は、設定の変更でも、Chrome のモードからの切り替えやコードの差し替えでも確かめない。使えないコードは最初の要求で断られて止まる（「断られたら止める」）。Worker の中継は期限から 24 時間までは通す
- **1 日の上限**: コードごとに、1 日（UTC）に使える額の上限を持つ。長い配信を止めず、コードが漏れたときの被害を抑えるため。既定は $60（`db.ts` の `DEFAULT_DAILY_LIMIT_USD`）で、翻訳先 3 つ、話し続ける最悪の約 $2.5/時間でも、1 人が 24 時間使い続けて届かない値にする（UTC で区切るので、1 人が 1 日に使えるのは最大 24 時間。1 つのコードを何人かで使うときは届くことがある）。おすそ分けした人がコードごとに変えられる。新しいコードの初期値はアプリが持ち（`DailyLimitField.svelte` の `DEFAULT_LIMIT`）、作るフォームはこの額を入れて開く。サーバーから読まない。おすそ分けを使わない人の通信を増やさず、オフラインでも額が見えるようにするため。変えるダイアログは実際に効く額（`effectiveLimitUsd`）を入れて開く。額が空や 0 以下ならエラーにし、新しいアプリは作るときも変えるときも額を明示して送るので、`NULL` のコードも設定を保存すると決めた額になる（止める・戻すだけなら上限は送らず、`NULL` のまま）。サーバーは今あるコードと古いアプリのために、DB の `NULL` を既定の額として扱い続ける（上限を送らない作成と、`null` を送る変更もそのまま受ける。一覧の `dailyLimitUsd` も古いアプリのために返す）。超えたら `giftKey` が 403 `limit` で断り、`GET /v1/gift` も同じように答える。Realtime は接続ごとに今日の残額をメモリに持ち、音声を受けるたびに引いて、なくなったら切る（close の reason は `limit`）。アプリは確定の字幕の 403 `limit`、Realtime の close の reason、ハンドシェイクの 403（本文は読みきれないので `X-Gift-Refusal` のヘッダーで見る）のどれでも字幕を止め、張り直さずに「今日の上限に達しました」と出す。今日の上限に達したコードは一時的に使えないだけなので、ギフトコードの保存は通す。日が変わると `codes.day` が変わるので、今日の分は 0 に戻る（履歴は持たない。月の集計は `code_usage` のまま）
- **断られたら止める**: 確定の字幕か Realtime のハンドシェイクが、送り直しても通らない答えで断られたら、字幕を止めて理由を操作パネルに出す（`share::refusal`。401 はギフトコードなら `GiftUnknown`、自分のキーなら `InvalidKey`。403 は `revoked`、`expired`、`limit`）。すぐ止めて、待っている確定の文と enrich は捨てる（断られた鍵へ送り直さない）。スタンダードなら Chrome も閉じる。回線の失敗、5xx、429、OpenAI の 403 では止めない（その発話の確定の文が抜けるだけ）。ギフトコードの 401 は理由を読まない。ハンドシェイクでは本文を読みきれずヘッダーもなく、Worker を通った OpenAI の 401 でも、おすそ分けした人に確かめてもらうのは同じなので、どちらも `GiftUnknown` にする。中継の 403 `expired` は期限の日時を返さないので、`GiftExpired` の `expiresAt` は `null` になることがある。Realtime の接続の途中で止められたときの close の reason は `limit` のほかは見分けられないが、張り直しのハンドシェイクで分かる
- **要求の速さの上限**: 額は応答が終わってから書くので、同時の要求は 1 日の上限では止まらない。`giftKey` が Workers の Rate Limiting（`wrangler.jsonc` の `GIFT_RATE`。キーはコードの id）で 1 コード 1 分 600 回に抑え、超えたら 429 `rate` を返す（403 のあとに見るので、止まる理由を隠さない。`GET /v1/gift` は数えない）。アプリの呼び出しは 1 発話につき最大 3 回（確定の文字起こし、翻訳と用語、ふりがな。`close` と `enrich`）で、Realtime は接続に 1 回だけ。話し続けた実測は 1 分 10 発話（sample02、29 秒で 5 発話）で 30 回。既定の区切りで最も速い 1 秒 1 発話（`MIN_SPEECH_MS` の 250ms + 無音 750ms）を続けても 180 回。辞書の読みは 40 語ずつ順に待つので 1 分に数回（20 秒ほどで 3 回）で、起動のときだけ。600 回は、実測の速さなら 20 人、理論上の最速でも 3 人が同時に話し続けて届かない（設定の最短の無音 200ms を全員が使って理論上の最速を続けると 1 人 400 回で、1 人半で届く）。1 回断られても、その発話の確定の文か翻訳が抜けるだけで字幕は止まらず（新旧どちらのアプリも）、辞書の読みは次の起動で問い直す。実際の上限はこうなる: 1 回の最大は約 $0.4 なので、1 コードで使われる速さは最大 1 分 $240。額を書くのは応答が終わったあとなので、1 日の上限を超える分は、いちばん長い応答が終わるまでの間に通った要求の分になる（32,000 トークンを 100 トークン/秒で約 5 分、推定。最悪で約 $1,200）。Rate Limiting は Cloudflare の場所ごとの近似の値で、多少超えて通ることがあり、場所を分けられるとその数だけ倍になる（厳密にするなら Durable Object だが、この用途では置かない）。Realtime の音声はこの回数に入らないが、接続が残額を持つので、実時間より速く送られても 1 つの接続では上限を超えない。残額は接続の始めと 30 秒ごとの確認で D1 から取り直すので、同時に張った別の接続の分は確認まで見えず、その間に使った分だけ超える
- **中継するもの**: Realtime（WebSocket。`/v1/openai/realtime`）、確定の文字起こし（`/v1/openai/audio/transcriptions` と xAI の `/v1/xai/stt`）、Responses API（`/v1/openai/responses`）。xAI の文字起こしは、アプリが送る欄（`model`, `language`, `keyterm`）とファイルだけを、ファイルを最後にして送り直す。単価の分からないモデル（`price.ts` の表にないもの）は通さない。Responses はアプリが使う項目（model・instructions・input・text・reasoning・max_output_tokens・stream）だけを渡し（有料のツールは通さない）、`store: false` にする（コードを使う人の字幕の文が、おすそ分けした人の OpenAI アカウントに保存されて見えないように）。`input` は文だけを受け（配列なら画像やファイルを渡せる）、本文は 256,000 文字まで（いちばん長い用語の判定で、辞書の全部を送って約 32,000 文字。その 8 倍）、`max_output_tokens` は 32,000 に丸める（アプリは送らないので必ず付ける。切れると JSON を読めないので、いちばん長い辞書の読みの 40 語で約 3,000 トークンの 10 倍にする。推論の分もここに入る）。1 回の最大は約 $0.4。確定の文字起こしはアプリが送る形（PCM・16kHz・16bit・モノラルの WAV）だけを受ける。Realtime はアプリが送る種類の文字のメッセージ（`session.update`・`input_audio_buffer.append`・`commit`・`clear`）だけを通す（バイナリのフレームも切る）。`session.update` は `type: "transcription"` と `format: { type: "audio/pcm", rate: 24000 }` を明示したものだけを受ける（料金は 24kHz・16bit の長さで数えるので、G.711 なら実際の 1/6 になる。`format` を省いた形はアプリが送らないので、OpenAI の既定に任せず切る）。単価の分かるモデルを `session.update` で決めるまで、ほかのメッセージを通さない
- **使われた額**: アプリの料金の数え方（13 章）と同じ単価で、コードごと・月ごとに D1 に足す。Realtime は送った音声の長さ（`input_audio_buffer.append` の PCM）を 30 秒ごとと切れたときに（30 秒ごとにコードがまだ使えるかも確かめ、止められていたり今日の上限に達していたりしたら接続を切る）、文字起こしは WAV の実際のバイト数から数えた長さを、Responses は応答の `usage` を数える（ストリームは流しながら写しを読み、最後の応答の `usage` を数える）。失敗した要求は数えない。おすそ分けタブにコードごとの額（今日と上限・今月・合計）を、料金タブにキー全体の概算（自分の分 + おすそ分けの分）を出す
- **保存の仕方**: キーは AES-GCM で暗号化して D1 に置き、戻す鍵は Worker の秘密の設定（`SHARE_KEY`）に置く。合言葉とコードは SHA-256 のハッシュだけを置く。中継する音声と文字は保存しない
- **表**: `issuers`（合言葉のハッシュ・合言葉を決める salt）、`issuer_keys`（人・会社・キーのハッシュ・暗号化したキー）、`codes`（コードのハッシュ・メモ・期限・止めたか・1 日の上限・今日の額）、`code_usage`（コード・月・額）。`server/src/schema.ts`
- **D1 の場所**: 作成のリクエストを出した場所の近くに作られるので、日本から `wrangler d1 create deadlock-jimaku` で作る（場所の指定は要らない）。中継はコードを確かめるたびに D1 を読むので、利用者（日本）に近いほうが遅れが少ない
- **テスト**: wrangler の `getPlatformProxy` で手元の D1 を立て、OpenAI と xAI への通信（`openai.ts`, `xai.ts`）は差し替える。Realtime の中継は偽の WebSocket で確かめ、v0.2.13 と同じメッセージを通すことを固定する（おすそ分けを持つ版の `stt.rs` と `llm.rs` の送る形は同じ）。公開中のアプリも同じ Worker を使うので、受ける形を狭めるときは古い版の形を確かめる
- **アプリの側**: 会社と送り先と鍵を `access::Access` にまとめる（自分のキーなら `https://api.openai.com/v1` か `https://api.x.ai/v1`、ギフトコードなら `{apiBase}/openai` か `{apiBase}/xai`。Worker は同じパスで中継するので、文字起こし・LLM の処理は送り先を替えるだけ）。自分のキーがあれば自分のキーを使う。ギフトコード・おすそ分けの合言葉・作ったコードは資格情報ストアに置く（作ったコードはサーバーがハッシュしか持たないので、あとでコピーできるようにこちらで覚える。Windows の資格情報マネージャーは 1 件の大きさに上限があるので、コードは 1 つずつ別の項目にする。覚えられなかったときは、作った直後のその場でだけコードを見せる）。ギフトコードは、動いていない字幕を始めるときだけ確かめる（`Slot::is_running`）。動いている間の作り直しでは確かめない。おすそ分けの状態（使われた額）は、起動したとき・キーを変えたとき・おすそ分けか料金のタブを開いたときに取り直す。自分のキーを消しても、有効にしたおすそ分けは見えたまま（止めたり無効にしたりできる）。合言葉は中核だけが使い、操作パネルには渡さない（`/api/share` などは中核が Worker を呼ぶ）。サーバーの根元は詳細設定の `apiBase`（既定 `https://api.jimaku.deadlock.jp/v1`。手元で試すときは `wrangler dev` に向ける）
- **コードを画面に出すとき**: 配信に映っても大丈夫なよう、先頭 4 文字のほかは伏せ、コピーと「表示」のボタンを付ける。ギフトコードの入力欄もパスワードの欄にする
- **確かめたこと**: 手元の `wrangler dev` を通して、録音のサンプルで Realtime・確定の文字起こし・用語と翻訳・ふりがなが動き、Worker が数えた額とアプリの料金の数え方が一致した（$0.0134）
