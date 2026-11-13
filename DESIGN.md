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

- 配信 PC は Windows が中心（Deadlock・OBS・本ツールを同じマシンで動かす。macOS と Linux の版も配る）
- ヘッドホンを使う（マイクにゲーム音が回り込まない）
- 音声認識: OpenAI（GPT-Live-Transcribe と GPT-Transcribe）か xAI（Grok Voice Transcribe。確定だけ）。3 章。確定は、OpenRouter の MAI-Transcribe 2（9.7）と、PC の中の Haas-ja（9.8）も選べる。Qwen-Audio-3.1-ASR-Flash は、途中の字幕（Streaming）にも確定にも選べる（3.4）
- 翻訳と用語: OpenAI GPT-6.1 Terra。詳しく選べば、用語を判定モデル（Decisions API、Jev、OpenRouter の判定モデル）、翻訳を翻訳専門のモデル（Qwen-MT-Flash、Hy-MT2）で別々に作れる（9.7）。辞書の漢字名の読みは GPT-6.1 Terra（どちらも推論なし。9.3）
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
| デスクトップ | **Tauri v2** | インストーラー（Windows は NSIS、macOS は dmg、Linux は AppImage・deb・rpm）、自動更新、トレイ常駐、Rust の中核 |
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

## 3. 音声認識: OpenAI と xAI（2026-11 時点のドキュメントで確認）

| 用途 | モデル ID | 料金 | 備考 |
|---|---|---|---|
| partial | `gpt-live-transcribe` | $0.017/分 | Realtime の文字起こしセッション（WebSocket）。サーバーは発話を区切らない |
| final | `gpt-transcribe` | $0.0045/分 | `/v1/audio/transcriptions`。精度が最も高い |
| final（ライト） | `grok-voice-transcribe-2.0`（xAI） | $0.10/時間 | `https://api.x.ai/v1/stt`。GPT-Transcribe の約 3 分の 1 の料金で、精度は同じか少し下（3.2） |
| （翻訳・読みも OpenAI `gpt-6.1-terra`。9.3） | — | — | スタンダードと OpenAI のモードは OpenAI のキー 1 つで済む。確定を Grok にすると xAI のキーも要る |

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

**合成音声で、用語を書けるかを測った**（2026-11-12。`experiments/tts_generate.py`、`crates/core/examples/stt_terms_eval.rs`、`experiments/results/stt_terms.json`）。上の録音は 2 本で、用語が数個しか出てこない。ゲーム実況風の 42 の発話（`experiments/data/stt_script.json`。辞書の公式名を 69 個。ヒーロー 31 人）を、ElevenLabs の Eleven v4 で、日本語の話者の声 3 つ（Aya、Hijiri、Kano）に読ませた（126 本、約 9 分）。読ませた文がそのまま正解になる。アプリと同じ部品（`stt::transcribe`）に、アプリと同じヒント（辞書の公式名）を渡した。合成音声ははっきり話すので、ふだんの配信の正確さより甘く出る（文字誤り率は、エンジンどうしを比べる目安）。

| | 用語を書けた（207 個中） | 文字誤り率 | 応答（中央値） |
|---|---|---|---|
| GPT-Transcribe | 191（92%） | 2.7% | 0.84 秒 |
| Grok Voice Transcribe | 186（90%） | 3.4% | 0.39 秒 |
| MAI-Transcribe 2 | 186（90%） | 3.3% | 0.63 秒 |
| Qwen-Audio-3.1-ASR-Flash | 182（88%） | 4.1% | 0.77 秒 |
| Haas-ja（Mac の GPU） | 177（86%） | 4.9% | 0.02 秒 |
| Chrome（声ごとに 1 本につなげて、偽のマイクで聞かせた） | 171（83%） | 6.1% | — |

- 書けなかった用語は、ほとんどが聞き違いではなく、書き方の違い。「ヴィスカス」を「ビスカス」（どのエンジンも 3 回とも）、「ヴァイパー」を「バイパー」、「ホリデイ」を「ホリデー」、「レディガイスト」を「レディーガイスト」、「ペイジ」を「ページ」、「シヴ」を「シブ」。用語のヒントを渡しても直らない（GPT-Transcribe がいちばん直す）。PC の中の一致（9.6）は書き方が同じでないと当たらないので、そこで、読みを比べる前に、同じ音の書き方をそろえるようにした（0.6.0-rc.13。`terms::fold`。ヴの行をバの行に、長音符を落とし、エの段のあとの「い」も落とす）。この測定の文に PC の中の一致を当てると、見つかる用語は 207 個中、GPT-Transcribe 191 → 194、Grok 186 → 196、Qwen-Audio 182 → 191、MAI-Transcribe 2 186 → 197、Haas-ja 177 → 186。短い名前（読みが 4 文字より短い「シヴ」「ペイジ」）は、ふつうの言葉（「支部」「ページ」）に当たるので、今までどおり比べない
- 「おひるね」は、どのエンジンも「お昼寝」と書く。同じ言葉なので、書けたと数えた（アプリは読みで一致させる）
- 順位は録音 2 本のときと同じ向き（GPT-Transcribe がいちばん良く、Chrome がいちばん悪い）。差は小さく、いちばん安い Qwen-Audio でも Chrome より良い
- 合成音声の読み違いは、耳では何本かしか確かめていない。どのエンジンも同じ書き方をした所（上の書き方の違い）は、音声の側が原因の可能性が残る

**PC の中の文字起こしを、同じ合成音声で比べた**（2026-11-13。0.7 に向けて。`experiments/stt_terms_eval_local.py`。speech.cpp 0.10.1 の `speech serve` に 1 発話ずつ送る。Apple M5 Pro の GPU）。Haas-ja より良いものはなかった。

| | 用語を書けた（207 個中） | 文字誤り率 | 応答（中央値） |
|---|---|---|---|
| Haas-ja v1.0（Q4_K。今使っているもの） | 177（86%） | 4.9% | 0.02 秒 |
| Qwen3-ASR-1.7B（Q8_0） | 157 | 5.8% | 0.19 秒 |
| 同 ＋ ヒーローの名前 44 個を `prompt` に | 165 | 6.5% | 0.24 秒 |
| 同 ＋ 辞書の公式名を全部 | 152 | 90%（話していない用語を書き並べる） | 0.74 秒 |
| Qwen3-ASR-1.7B（Q4_K） | 151 | 7.4% | 0.14 秒 |
| Parakeet-TDT_CTC-0.6b-ja | 149 | 6.9% | 0.02 秒 |
| Qwen3-ASR-0.6B | 125 | 11.6% | 0.09 秒 |
| ReazonSpeech v2 | 84 | 37.9% | 0.04 秒 |

- 用語を教えられる Qwen3-ASR に期待したが、ヒーローの名前を渡しても、何も渡さない Haas-ja に届かない。全部渡すと壊れる（Haas-ja と同じ）
- 途中の字幕に向くモデル（Nemotron 3.5 Streaming、Voxtral Realtime）は、speech.cpp に入っていないので測っていない

**Whisper は、ヒーローの名前を渡すと Haas-ja を超える**（同じ日、同じ道具。whisper.cpp 1.9.5 の `whisper-server`。`prompt` に名前を「、」でつないで渡す）。

| | 何も渡さない | ヒーローの名前 44 個 | 辞書の公式名を全部 | 応答（中央値） |
|---|---|---|---|---|
| Whisper large-v3（3.1 GB） | 182（文字誤り率 4.0%） | **202**（3.0%） | 182 | 0.49〜0.54 秒 |
| 同 q5_0（1.08 GB） | — | **203**（2.9%） | — | 0.49 秒 |
| Whisper large-v3-turbo（1.6 GB） | 180（4.4%） | **194**（4.4%） | 181 | 0.22〜0.24 秒 |
| 同 q5_0（574 MB） | 179（4.5%） | 194（4.2%） | — | 0.22〜0.24 秒 |
| kotoba-whisper v2.0（1.5 GB。q5_0 も同じ） | 164（7.0%） | 0（何も書かない） | — | 0.21 秒 |

- large-v3 にヒーローの名前を渡した 202 は、会社のモデルで一番良かった GPT（191）より上。turbo の 194 も上
- 全部渡しても効かない（Whisper が `prompt` として読むのは終わりの 224 トークンまで。応答は 2.4 秒以上に延びる）。渡すのは、ヒーローの名前くらいの量にする
- 小さくした turbo（q5_0）は、精度も速さも元と変わらない
- 応答は Haas-ja（0.02 秒）の 10〜25 倍。Windows の GPU での速さと、ゲームへの影響はまだ測っていない

**CPU だけで動かす場合と、SenseVoice**（同じ日。SenseVoice-Small は sherpa-onnx 2.0.1 で、CPU の 4 スレッド。`experiments/stt_terms_eval_sherpa.py`。Haas-ja と Parakeet は speech.cpp に `--device cpu`）。

| | 用語を書けた（207 個中） | 文字誤り率 | 応答（中央値） |
|---|---|---|---|
| Haas-ja v1.0（Q4_K）を CPU で | 177 | 4.8% | 0.13 秒 |
| SenseVoice-Small（0.93 GB） | 153 | 6.1% | 0.05 秒 |
| 同 int8（0.24 GB） | 149 | 6.2% | 0.05 秒 |
| Parakeet-TDT_CTC-0.6b-ja を CPU で | 150 | 6.8% | 0.14 秒 |

- Haas-ja は、CPU だけでも精度が変わらず、1 発話 0.13 秒で返る。GPU をゲームに空けたい人は、CPU で動かせる
- SenseVoice は CPU で Haas-ja の 2.5 倍速いが、用語は 24 個少ない。途中の字幕に使うなら、Haas-ja を CPU で動かすほうが良い（speech.cpp は、話している途中の文字を返す口 `/v1/realtime` も持っている。まだ試していない）
- sherpa-onnx の配布物のうち `…-2025-09-09` は、広東語向けに鍛え直した別のモデル（日本語を書けない）。測ったのは `…-2024-07-17`

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

前のライトのモード（確定は Grok、翻訳と用語は GPT-6 Luna、ふりがなは PC の中。0.6.0 で翻訳の選択肢から Luna を消した。9.6）は、Grok が約 26 分 × $0.0017 = 約 $0.04、Luna の翻訳が Terra の約 6 分の 1 で約 $0.05、合わせて **約 $0.1/時間**（推定。アプリでは測っていない）

「話している途中から字幕を出す」をオフにすると Realtime につながず、確定の字幕だけを出す。途中の字幕の分がなくなり、sample02 では料金が約 46% 下がった（字幕は話し終えてから出るので、長い発話ほど最初の字幕が遅くなる。sample02 の最初の発話で 6.5 秒が 11.3 秒に）。

アプリで数えた実測（sample02、29 秒・5 発話）: 途中の字幕 $0.0058（21 秒）、確定の字幕 $0.0015（20 秒）、翻訳 $0.0024（1,771 トークン）、ふりがな $0.0026（1,333 トークン）、合計 $0.012。話し続けた 1 時間に直すと、sample01 で約 $1.9、sample02 で約 $1.5。LP には「話し続けて約 $2」と書く。操作パネルの「料金」タブに出す（13 章）。

「話した言葉を出す」をオフにすると、途中の字幕とふりがなが動かない。sample02 の実測で $0.0141 が $0.0062 になり、約 56% 下がった（最初の字幕は 2.7 秒が 15.0 秒に。最初の長い発話を話し終えて、翻訳が届くまで出ないため）。1 時間の目安は約 $0.5。

Voxtral のときはリアルタイムに無音も含めて送り続けていた（delta を届いた時刻で発話に振り分けていたので、途中を抜けなかった）。OpenAI では発話ごとに commit するので、話していない間は送らない。

### 3.4 Qwen-Audio-3.1-ASR-Flash（`stt/qwen.rs`）

測った結果（上の表）が、途中の字幕は GPT-Live-Transcribe の約 20 分の 1、確定は Grok の約 7 分の 1 の料金で、精度は同じくらいだったので、0.6.0-rc.3 で選択肢に入れた。鍵は QwenCloud のシンガポールのもの（翻訳の Qwen-MT-Flash と同じ）。OpenAI 互換の口では使えず、独自の口に送る。

- **確定（`qwen-audio-3.1-asr-flash`）:** `POST /api/v1/services/aigc/multimodal-generation/generation` に、発話の WAV を base64 で 1 回送る。用語は、前の発話の文として 400 文字まで添える（ヒーローの名前から）。料金は答えのトークン数で数える。22 秒の録音で約 5 秒かかった（発話はふつうもっと短い）
- **途中の字幕（`qwen-audio-3.1-asr-flash-streaming`）:** WebSocket（`/api-ws/v1/inference`。`run-task` → 16kHz の PCM のバイナリ → `finish-task`）。OpenAI の Realtime と違い、文の区切りはサーバーが決める。アプリの発話（VAD の区切り）と合わせるために:
  - 話している間の音声だけを流し、流したサンプル数を数える。発話を開いた位置を覚えておく
  - 届く文には、文字ごとの時刻（流した音声の先頭からのミリ秒）が付く。その時刻に開いていた発話に、文字を割り当てる。サーバーの文はアプリの発話をまたぐことがある（発話の終わりで文が終わるとは限らない。実測）ので、文ではなく文字で分ける
  - 文は書きかけのうちから何度も届き、届くたびに文字が置き換わる。だから字幕の処理には、追記ではなく「発話の今の文字の全部」を渡す（`Delta::whole`）
  - 発話を閉じたら 0.6 秒の無音を流す（次の発話とつなげない）。音声を 23 秒送らないと切られる（実測）ので、黙っている間は 10 秒ごとに 0.1 秒の無音を流す（マイクの音は送らない）
  - 使用量は接続を始めてからの合計で届くので、増えた分を数える
- 確かめ方: `cargo test -p overlays-core qwen -- --ignored --nocapture`（本物に録音を送る。途中の字幕は、録音を 2 つの発話にして間を 30 秒空ける）。最初の文字は、つなぎ始めてから約 2 秒（接続に 1.4 秒）
- 用語のヒントは、途中の字幕には渡せない。単価は料金の比較サイトの値（100 万トークンで、確定は入力 $0.15・出力 $0.47、途中の字幕は入力 $0.93・出力 $0.70。音声 1 秒が入力約 11 トークン）

### 3.5 発話の区切りを会社に任せるか（任せない）

区切りはアプリの VAD（`segment.rs`。無音 0.75 秒）が決める。会社に任せる形（「自動」）も測ったが、良くならないので入れていない（2026-11、`experiments/segment_eval.py` と `crates/core/examples/segments.rs`。録音を実時間で流す）。

| | sample01（22 秒、7 文） | sample02（29 秒、7 文） | 区切りが届くまで（話し終えてから） |
|---|---|---|---|
| アプリの VAD（無音 0.75 秒） | 3 つ | 5 つ | 0.75 秒（設定の値） |
| アプリの VAD（無音 0.3 秒） | 6 つ | 8 つ | 0.3 秒 |
| Qwen-Audio-3.1-ASR-Flash-Streaming の文 | 2 つ（2 つ目は 15 秒） | 5 つ（アプリの 0.75 秒と同じ位置） | 1.0〜1.6 秒 |
| Chrome の確定 | 3 つ（アプリの 0.75 秒と同じ位置） | 5 つ（同じ位置） | — |
| GPT-Live-Transcribe の `turn_detection` | 使えない（「Turn detection is not supported for this transcription model」） | | |

- Qwen の文の区切りも、文の意味ではなく無音の長さで決まっている（1 秒ほどの無音では切れず、アプリより長い無音を待つ）。句点は文の中に付くが、そこでは切れない。だから任せても、区切りが粗くなり（15 秒の文）、届くのが遅くなるだけ
- Chrome の確定も、アプリの 0.75 秒とほぼ同じ位置で切れる
- 文の終わりで切りたいなら、会社に任せるのではなく、途中の文字の句点を見てアプリが切る形になる（Qwen は文字ごとの時刻と句点を返す）。どの会社でも同じに効くが、途中の字幕の誤りで切れ方が変わるので、入れるなら測ってから

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
│  │     ├─ segment.rs        # 発話の区切り（VAD）
│  │     ├─ stt.rs            # 音声認識の共通の型と、会社への振り分け
│  │     ├─ stt/              # 途中の字幕を発話に合わせる（openai.rs、qwen.rs）
│  │     ├─ local.rs          # PC の中の文字起こし（speech.cpp を立てる）
│  │     ├─ decide.rs         # 用語の候補出しと、判定モデルの確かめ
│  │     ├─ mt.rs             # 翻訳専門のモデル
│  │     ├─ enrich.rs         # 翻訳（OpenAI）の呼び出し・出力の検証
│  │     ├─ subtitles.rs      # パイプライン（Segment の状態遷移）
│  │     ├─ chrome.rs         # 聞き取りのページを Chrome で開く
│  │     ├─ annotate.rs       # 読みと用語を付ける（専用のスレッド）
│  │     ├─ reading.rs        # 形態素解析（vibrato + 話し言葉の UniDic）
│  │     ├─ terms.rs          # 速い用語の判定
│  │     ├─ assets/           # deadlock-api → glossary + アイコンのキャッシュ
│  │     ├─ rank.rs           # ランクのポーリング
│  │     ├─ lang.rs           # 言語の対応表
│  │     ├─ store.rs          # 状態ストア + broadcast
│  │     ├─ server.rs         # axum のルーティング
│  │     └─ types.rs          # ts-rs で TS に出力する型
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
- **読みの辞書は、vibrato と「現代話し言葉 UniDic」（0.6.0-rc.9 から。`reading.rs`）。** 0.6.0-rc.8 までは Lindera + IPADIC だった。辞書を 5 つ比べて決めた（2026-11。辞書だけで読んで、正しく読めた文の数。難しい 60 文は `experiments/data/furigana_cases.json`、ふだんの文は用語のテストの 63 文、短い話し言葉は助詞を省いた言い方 40 個）:

  | 辞書 | 難しい 60 文 | ふだんの 63 文 | 短い話し言葉 40 個 | 大きさ |
  |---|---|---|---|---|
  | IPADIC（Lindera に同梱） | 42 | 62 | 33 | 45 MB |
  | IPADIC NEologd | 44 | — | — | 701 MB |
  | Sudachi（sudachi.rs、small。core と full、A・B・C 単位も同じ結果） | 49 | 61 | 34 | 115 MB |
  | 現代話し言葉 UniDic 2025.12（表をそのまま。MeCab と同じ結果） | 53 | 63 | 38 | 1,053 MB |
  | 同じ辞書を vibrato の小さい形にしたもの（**これを使う**） | 51 | 63 | 38 | 74 MB（圧縮して 12 MB） |

  - 書き言葉の辞書（Sudachi、書き言葉の UniDic）は、助詞を省く話し言葉を名詞の連続として読む（「ヘイズ強い」の「強い」を名詞の「しい」、「弾速い」を「弾速」「い」、「上行こう」を「上行」「こう」）。語のつながりやすさが、新聞や Web の文で調整されているため。IPADIC は語彙が少ないぶん引っ張られにくいが、数と人数（二人、一人、一回）と「金（かね）」を読み違える。話し言葉の UniDic は、話し言葉のコーパスで調整されていて、どちらも読める
  - 話し言葉の UniDic の配布物は MeCab 用で、語のつながりやすさの表（matrix）が 1 GB ある。vibrato は、この表を、もとになった規則（`feature.def`）と重み（`model.def`）の形で持てる（`vibrato::mecab::generate_bigram_info`。値をその場で計算するので 1 文 0.03 → 0.12 ミリ秒に遅くなるが、字幕には響かない）。素性も品詞と読みだけにして、74 MB、圧縮して 12 MB。計算し直した値がわずかに違うので、難しい文で 2 文だけ落ちる（「弾」「他」「角」を読み違え、「大分」が直る）。今の辞書（45 MB）より小さいので、ダウンロードにせず、アプリに入れる（`crates/core/dictionary/unidic-csj.dic.gz`。初めて使うときに 0.7 秒かけて広げる）
  - 辞書は `cargo run --release -p overlays-core --example build_dictionary -- ソースのフォルダ` で組み立てて、リポジトリに入れる（ソースは 3 GB あるので、ビルドのたびには作らない。配布元が新しい版を出したら作り直す）。ライセンスは修正 BSD を選ぶ（`notices/unidic.txt`）
  - Sudachi（sudachi.rs）への入れ替えも最後まで作って測ったが、上の読み違いが出るのでやめた。辞書を初回にダウンロードする形も、その中で作った（`download.rs`。今は PC の中の文字起こしだけが使う）
  - 読みの数字は変（「10 秒」を「いちぜろびょう」）だが、数字にはふりがなを付けないので出ない
  - 用語の一致（9.6）は、辞書の区切りを使う。新しい辞書は、知らないカタカナの語を知っている語に割ることがある（「レムソン」を「レム」「ソン」）ので、カタカナが続く途中では用語にしない決まりを足した（`terms::whole`）。用語のテストの 63 文で、PC の中の一致が正解と同じになる文は 25 → 27（`annotate::tests::local_terms_on_the_cases`。「アポロチョコ」「アイビーリーグ」の誤った一致が消え、悪くなった文はない）
  - 判定モデルに選ばせる形（下）の候補の出し方も替えた。vibrato は上位の分け方を返さないので、同じ文字で読みが 2 つ以上ある語の表（`dictionary/homographs.tsv.gz`、約 1.7 万語）を辞書から作っておき、辞書の選んだ語ごとに、表にある別の読みを候補にする（`Furigana::alternatives`）。表からは、複合語の中でだけ出る濁った読み（「辛い」の「がらい」）、固有名詞の読み（「金」の「きむ」）、めったに使わない読み（いちばんよく使う読みより語のコストが 5000 より大きいもの。「何」の「なぬ」、「大会」の「だいえ」）を外す。数と、その直後の数え方は聞かない（辞書は「一発」「三日」「四人」「二個」を組で正しく読むが、1 語ずつ選ばせると「いっはっ」「さんひ」「よんにん」「にいこ」に崩れた）。結果（Perplexity Decider、各 3 回）: 難しい 60 文は、辞書だけで 180 回中 153 回が正しく、選ばせると 172 回（直す 21、壊す 2。壊したのは「何か」を「なんか」と読んだ 1 文で、話し言葉としては正しい。残る間違いは「一日中」と「弾（たま）」）。ふだんの 63 文は、辞書だけで 189 回すべて正しく、選ばせても変わらない（聞いたのは 57 回）。前の辞書では、選ばせて 165 回だった
- **Chrome の中の Gemini Nano に、同じ候補から選ばせる形も測った（2026-11。まだ入れていない）:** 候補はアプリが判定モデルに渡すものと同じ（`examples/readings.rs` で書き出し、`experiments/chrome_eval.mjs choose` が Prompt API に聞く。答えは候補の番号に決める。日本語の文字を答えの決まりに入れると、Prompt API がエラーを返した）。難しい 60 文（180 回。辞書だけで 153）は 157 が正しい（直す 12、壊す 8。「金（かね→きん）」「上から（うえ→じょう）」「上方修正（→かみがた）」を壊す）。Perplexity Decider は 172（直す 21、壊す 2）。ふだんの 63 文（189 回）は、聞いた 60 回で 1 つも壊さない。1 問 0.27 秒。モデルは約 4 GB で、Chrome が初回に取る。前に測った「読みを自由に書かせる形」（直す 6、壊す 26）よりずっと良いが、難しい文での得は小さい（+4）
- **0.6.0-rc.7 から: 辞書の読みを LLM に直させる。** 文を辞書（Lindera）で語に分けて読みを付け、漢字を含む語と辞書の読みを、文と直前の発話と一緒に LLM に渡して、文に合わない読みだけを直させる（`furigana::ask`）。答えのうち、ひらがなでない読みと、語のかなに合わない読み（「避け」に「さける」）は使わず、その語は辞書の読みのまま。下の「文全体を LLM に読ませる」形（0.6.0-rc.6 まで）と同じ 60 文で比べると、正しく読めた回数・使えない答え・応答の中央値・180 回の料金は、Terra が 178・2・4.3 秒・$0.21 から 180・0・1.9 秒・$0.086、Luna が 167・11・4.9 秒・$0.030 から 172・0・3.2 秒・$0.012（OpenRouter を通した値。OpenAI に直接聞くと Luna は 1〜2.5 秒）。文全体を読ませる形は、答えが文と合わないとその文のふりがながすべて消えたが、直させる形は辞書の読みが残る。モデルは GPT-6.1 Terra（`RubySource::Llm`。`models.readings`）か GPT-6 Luna（`RubySource::Luna`。Terra の約 7 分の 1 の料金）。辞書の漢字名の読み（8.2）は今までどおり Terra に読ませる
- 原文の読みは文ごとに LLM（辞書の名前の読みと同じ `models.readings`、推論なし）に聞く（`furigana.rs`、9.2）。文脈で読みが決まる語（一人・上方修正・金・一日中）や造語も正しく読む。形態素解析（Lindera + IPADIC）は造語や文脈に弱く（「上方(かみがた)修正」「金(かね)」）、併用すると実装が増えるだけなので使わない。読みが取れなかった文はふりがななしで出す（ふりがなは補助なので困らない）
- 直前の発話を文脈に付ける。比較（`experiments/furigana_context.py`、文脈で読みが変わりうる短い発話と対照の 10 件 × 5 回。文脈はアプリと同じく 3 発話で、1 つは関係のない雑談）: 文だけでも金(かね)・一日(ついたち)・辛(つら)い・角(かど)・一枚上手(うわて)は正しく、誤ったのは「人気ないね」の人気(にんき)だけだったが、文脈（「このレーン誰もいない」）を付けると 5/5 で ひとけ になった（全体で 45/50 → 50/50）。文脈を付けても区切りが崩れる答えはなかった
- **辞書の読みを直させるモデルの比較（2026-11 に測った。小さいモデルは入れない）:** 読み分けの要る語を含む 60 文（`experiments/data/furigana_cases.json`。辞書の弱いところを狙った文で、辞書だけだと 42 文が正しい）の、辞書（Lindera）の語と読みを渡して、文に合わない読みだけを直させた（各 3 回で 180 回。辞書が正しいのは 126 回）。Chrome の中の Gemini Nano（Prompt API。`experiments/chrome_eval.mjs furigana`）は、間違いの 54 回のうち 6 回を直し、正しい 126 回のうち 26 回を壊した（送り仮名まで含めた読みを返す。「避け」に「さける」）。正しく読めたのは 106 回で、辞書だけより悪い。応答の中央値は 0.57 秒。同じ問いを OpenRouter のモデルに送ると（`experiments/furigana_fix.py`。正しく読めた回数・直した・壊した・応答の中央値・180 回の料金）: GPT-6.1 Terra 180・54・0・1.9 秒・$0.086、GPT-6 Luna 172・46・0・3.2 秒・$0.012、Gemma 4 26B 154・34・6・0.6 秒・$0.003、Gemini 2.5 Flash Lite 143・29・12・0.8 秒・$0.005、Gemma 3 12B 53・4・77、Gemma 3 4B 88・0・38。Qwen3.5-Flash は答えの形が崩れて測れなかった。壊さないのは Terra と Luna だけで、Luna は Terra の約 7 分の 1 の料金だが遅い。この結果から、Terra と Luna を選択肢にした（上）
- **今の辞書（話し言葉の UniDic）で、3 つの作り方を測り直した（2026-11-12。`examples/furigana_eval.rs`。難しい 60 文を 3 回ずつで 180 回、辞書だけで 153）:** Perplexity Decider に選ばせる形は 171（直す 21、壊す 3、聞けなかった 2）・応答の中央値 0.47 秒・$0.0005。GPT-6.1 Terra に直させる形は 180（直す 27、壊す 0）・1.5 秒・$0.071。GPT-6 Luna に直させる形は 179（直す 26、壊す 0）・1.2 秒・$0.0071。下の記録（前の辞書、OpenRouter を通した Luna は 172・3.2 秒）より、Luna がよく直し、速い。操作パネルの選択肢の説明は、この値を出す
- **辞書の候補から判定モデルに選ばせる形（2026-11 に測り、0.6.0-rc.8 でベータのふりがなにした。`RubySource::Decider`、`furigana::choose`）:** ここからは、前の辞書（Lindera + IPADIC）で測ったときの記録。今の辞書での候補の出し方と結果は、上の「読みの辞書」に書いた。 まず文全体の読みを選ばせる形で測った。 Lindera の上位 20 個の分け方（`segment_nbest`）から文全体の読みを集めると、1 文あたり平均 5.6 通りで、同じ 60 文のうち 58 文、辞書が間違える 18 文のうち 16 文で、正しい読みが候補に入っている（そのときの測定の例は、辞書を替えたときに消した。入らないのは「一日（ついたち）」と「四人（よにん）」）。この候補を、用語の判定と同じ口（OpenRouter の `/systemone`）で判定モデルに選ばせた（`experiments/furigana_choose.py`。各 3 回で 180 回。正しく読めた・直した・壊した・応答の中央値・180 回の料金）: Perplexity Decider V1.2 162・42・6・0.47 秒・$0.0007、Nace Drex Max 158・35・3・0.63 秒・$0.002、Microsoft Decision 1 140・26・12・0.44 秒・$0.001。辞書だけの 126 より良く、Luna に直させる形（172・46・0・3.2 秒・$0.012）より少し落ちるが、速さは約 7 倍、料金は約 17 分の 1。Perplexity Decider が壊した 6 回は 2 文で、「弾（たま→だん）」と「空いてる（あいてる→すいてる。どちらとも読める）」。候補の中からしか選べないので、語の範囲を壊す答えは出ない。ベータの中継（用語の判定と同じ口、同じモデル）にそのまま載る（Worker は変えていない）。**入れた形**は、文全体ではなく、読みが分かれるところごとに聞く（`reading::Furigana::alternatives`。上位 20 個の分け方のうち、いちばん良い分け方と読みが違う範囲を集め、重なる範囲は 1 つにまとめる）。候補は「同じ語の別の読み（辛い）」と「語をまとめた分け方（一＋人 → 一人）」だけで、辞書の語をさらに割った分け方は出さない（出すと「火力」を「ひりょく」、「大会」を「だいかい」と選ぶことがあった。ふだんの配信の文 63 個で、直す 1・壊す 2 が、直す 1・壊す 0 になった）。選択肢には続くかなを 4 文字まで付ける（語だけを見せると、「来てね」の「来」を「こ」と選んだ）。答えの確からしさで絞る形は、壊す数より直す数のほうが大きく減ったので入れていない（しきい値 0.8 で、60 文のうち直す 16 → 12、壊す 2 → 1）。入れた形の結果（各 3 回）: 難しい 60 文は、辞書の間違い 54 回のうち 48 回を直し、合っていた 126 回のうち 9 回を壊した（3 文。「弾（たま→だん）」「大分（だいぶ→だいぶん）」「何か（なにか→なんか）」。後の 2 つは、そうも読む）。正しく読めたのは 165 回（辞書だけは 126、Luna に直させる形は 172）。ふだんの文 63 個（`experiments/data/terms_cases.json`。GPT-6.1 Terra に直させた読みを基準にした）は、189 回のうち直す 3、壊す 0（聞いたのは 54 回。読みが分かれるところのない文は聞かない）。応答の中央値は 0.47 秒、料金は 180 文で $0.0006（`crates/core/examples/furigana_eval.rs`）。確定の文には、まず辞書の読みを付けて出し、答えが届いたら替える（聞けなかった文は辞書の読みのまま。LLM に直させる形も同じ）

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
   - キーがなければ LLM の読みはない。用語の判定は、読みの辞書（6.4）が名前を読んだ結果を使う（9.6）。手で保守する読みは持たない
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

1. 日本語で漢字を含む文なら（ふりがなを出す設定のときだけ）、辞書の読みを LLM（`models.readings`。ふりがなが Luna なら GPT-6 Luna）に直させる（2 と同時に。6.4）。直前 3 発話を文脈に付ける。答えが使えない語は辞書の読みのまま。聞けなかったら（回線、タイムアウト）PC の中で付けた読みを使う
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
    - この結果から、翻訳と用語の選択肢に Luna を足した（安いが用語の判定は弱い、と添える）。おすすめは Terra のまま。ふりがなに Luna を使えるかは測っていない。0.6.0 で、この選択肢は消した（9.6）
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

字幕の作り方を 3 つから選ぶ。新しく使い始める人の既定は `chrome`（インストール直後からキーなしで字幕が出せる）で、おすすめは `standard`。Chrome のモードで自分の OpenAI のキーか、OpenAI に使える有効なギフトコードを保存すると（`panel.standardReady`）、モードの欄に「スタンダードにする」の提案を出し、押して切り替える（設定を黙って変えない）。提案は閉じられて、閉じたら保存して再び出さない（`hybridHintClosed`。モードの欄から選べるので困らない）。スタンダードを選んで鍵がない人は「キーなしで始める（Chrome）」で、これも押して切り替える。途中と確定の文字、翻訳、ふりがな、強調を何で作るかは、工程ごとの選択（`Engines`）と設定から `Settings::plan` が 1 か所で決める（README の表と同じ。テストに同じ値を書く）。Chrome が要るかもここから分かる。OpenAI のモードの途中の字幕にも読みと用語を付ける。

**モードの整理（0.6.0-rc.5、rc.6）:** 操作パネルのモードは、ベータ・Chrome と、最高精度（`Engines::MODES`）。rc.5 から並べていた「ローカル」（途中の字幕も確定も Haas-ja）は、モードから外した（0.6.0-rc.11）。Windows の実機で、カタカナの用語に弱く（Grok のほうがずっと良い）、途中の字幕で CPU を使うので、薦める組み合わせではなかった。用語と翻訳は外に出るので、名前も誤解を招いた。Haas-ja は工程ごとの選択に残る（9.8）。`beta` のモードは、画面では「おすすめ」と出す（0.6.0-rc.11。コードと設定の名前は `beta` のまま。この文書の「ベータのモード」も同じもの）。「ベータ」は無料で出している期間の名前で、字幕の中身を言っておらず、試験中で不安定とも読め、v1.0 で有料にすると名前が合わなくなる。「ベータ期間中は無料」は、名前の横のバッジと説明で伝える。前のライトと OpenAI は、ベータの無料の提供（17.2）を始めてからは薦める理由がなくなった（ライトより、ベータのほうが安くて精度も上。Chrome を使わない組み合わせは、工程ごとの選択で組める）。前のスタンダードは「最高精度」として 1 つ残す。用語の正解が 189 回中 186（ベータの組み合わせは 178）、文字誤り率が 0.8% と 3.9%（Qwen は 2.4〜3.2% と 3.9〜4.9%）で、差は数ポイントだが、確定の字幕が約 1.5 秒遅く、費用は約 17 倍（9.7、3.4）。お金を払ってでも良い字幕を出したい人の行き先で、0.5 から使っている人が迷わず、サブスクの上位プランの形にもなる。選択肢は工程ごとの選択に残っている（`Engines::LIGHT` などの定数は、テストで組み合わせの決まりを確かめるために残す）。自分で組んだ組み合わせは「カスタム」として 1 つ覚える（`Settings::custom_engines`。モードを試したあとに戻れる。名前を付けて何個も保存する機能は、使う人がいないので作らない）。工程の選択肢が増えてダイアログに収まらなくなったので、「モード」のタブ（`EnginesTab.svelte`）にした。上にモード、下に工程ごとの一覧（名前、料金、要る鍵、ひとこと）を置く。モードを選ぶのはこのタブだけで、「マイクと翻訳」のタブは今のモードと「変える」のボタンだけを出す（同じ選択を 2 か所に置かない）。**GPT-6 Luna を翻訳の選択肢から消した（0.6.0-rc.7）:** 用語と翻訳を Luna に同時に頼む形は、用語の正解が 189 回中 142、応答が 2.2 秒、189 回で $0.027。判定モデルと翻訳専門のモデルに分けた形（Decisions → Qwen-MT-Flash は 177・0.55 秒・約 $0.02、ベータの Perplexity Decider → Hy-MT2 は用語が 178）が、精度も速さも料金も上（9.7）。残る利点は OpenAI のキー 1 つで安く済むことだけで、ベータが無料の間は意味がない。保存されている `gpt6Luna` は、ベータの用語と翻訳（Perplexity Decider と Hy-MT2）として読む（`SavedEngines`。0.5 のライトの人。文字の送り先が OpenAI から開発者の中継に変わるので、リリースノートに書く。ベータのコードがまだない人には、鍵が要るという案内が出るので、「モード」のタブで選び直してもらう）。Decisions API のモデルとしての `gpt-6-luna` は変わらない。Chrome を使わずにすべてクラウドで作る組み合わせ（途中の字幕も確定も Qwen）は、途中の字幕の中継を Worker に足したので（17.2）、ベータのコードで選べる。モードとして並べるかは決めていない

**工程ごとの選択（`engines`）:** 設定には、モードの名前ではなく、工程（途中の字幕・確定の文字起こし・用語・翻訳・ふりがな）ごとに何で作るかを持つ。モードは決まった組み合わせ（`Engines::CHROME`・`LIGHT`・`STANDARD`・`OPENAI`）で、操作パネルの「詳しく選ぶ」のダイアログ（`EnginesDialog.svelte`）から工程ごとに変えられる。人それぞれの事情で値段と精度を選べ、料金の内訳も見え、OpenAI 以外のモデルも工程の選択肢として足せる。選んだ組み合わせがどのモードにも当たらなければ、モードの欄はどれも選ばず、有料の工程を書き出す。ふだんの人は 4 つのモードから選ぶだけで済むよう、工程の選択はダイアログに置く。0.4 までの設定ファイルは `load` が `speechEngine` と `liveSubtitles` から置き換える（0.3 より前の `speechEngine` のない設定ファイルは `openai` のまま。Chrome のウィンドウを黙って開かない）。鍵が要るかは工程の選択だけで決める（`Engines::paid`。有料の工程が 1 つでもあれば要る。どの会社の鍵かは下の「組み合わせの決まり」）。翻訳先やふりがなの表示で決めると、表示を切り替えるたびに字幕の処理を始め直すことになるため。

| モード | 途中の字幕 | 確定の字幕・翻訳・用語 | 特徴 |
|---|---|---|---|
| `beta`（画面の名前は「おすすめ」） | Chrome（Google）。途中の訳は Translator | 確定の文は Qwen-Audio、用語は Perplexity Decider、翻訳は Hy-MT2、読みは PC の中 | ベータ期間中は無料で、鍵が要らない（開発者のサーバーが中継する。17.2）。音声は Google と Qwen に送られる |
| `chrome` | Chrome（Google） | 確定の文は Chrome、翻訳は Translator、読みと用語は PC の中 | 無料、API キー不要。音声は Google に送られる。翻訳は Chrome の中で動く |
| `best`（最高精度。0.5 までのスタンダード） | Chrome（Google）。途中の訳は Translator | OpenAI（GPT-Transcribe、GPT-6.1 Terra） | 用語の判定と文字起こしがいちばん正確。OpenAI の鍵が要る。約 $0.6/時間。確定の字幕は 1〜2 秒長くかかる。音声は Google と OpenAI に送られる |

| 工程 | 無料 | 有料 |
|---|---|---|
| 途中の字幕 | Chrome（Google）。途中の訳は Translator | GPT-Live-Transcribe か Qwen-Audio-3.1-ASR-Flash-Streaming（3.4。どちらも途中の訳なし）。「出さない」も選べる |
| 確定の文字起こし | Chrome（発話の区切りも Chrome が決める）。PC の中の Haas-ja（9.8。日本語だけ。アプリのマイクと VAD で区切る） | GPT-Transcribe、xAI の Grok Voice Transcribe、Qwen-Audio-3.1-ASR-Flash（3.4）、OpenRouter の MAI-Transcribe 2（アプリのマイクと VAD で区切る。3.2） |
| 用語（翻訳が GPT でないとき） | 辞書の一致（`terms.rs`、日本語だけ） | PC の中の候補を判定モデルに確かめさせる（9.7。日本語だけ）。OpenAI の Decisions API（GPT-6 Luna）、TypeSafe AI の Jev、OpenRouter の Drex Max・Microsoft Decision 1・Perplexity Decider |
| 翻訳 | Chrome の Translator | 翻訳専門のモデル（9.7。Qwen-MT-Flash、OpenRouter の Hy-MT2）か、GPT-6.1 Terra（用語も同時に判定する。9.3） |
| ふりがな | 辞書（vibrato + 話し言葉の UniDic。日本語だけ。6.4） | 辞書の候補から Perplexity Decider に選ばせる（ベータの中継で使える）か、辞書の読みを GPT-6.1 Terra・GPT-6 Luna に直させる（6.4。日本語だけ） |

**組み合わせの決まり:** 今の仕組みで作れない組み合わせは `Engines::sanitized` が直し、操作パネルでは選べなくする（`frontend/src/lib/engines.ts` の `unavailable`）。

- 確定の文字起こしが Chrome なら、途中の字幕も Chrome。発話の区切りを Chrome の確定で決めるので、アプリはマイクを開かない（`chrome::run`）
- Chrome の翻訳は Chrome のページの中で動くので、途中の字幕が Chrome のときだけ選べる。途中の字幕が Chrome でないのにページを開くと、使わない音声認識まで動いて音声を Google に送ってしまう。操作パネルでは、Chrome の翻訳を使っている間は途中の字幕を Chrome から変えさせない（変えると翻訳が OpenAI に直り、黙って料金がかかる）
- 用語と翻訳は、訳文の名前を用語の判定の結果から作る（LLM は訳文の用語を参照 `⟦hero:13⟧` で書き、ほかは用語を英語の名前に置き換えてから訳す）。だから翻訳を GPT にしたら、用語も同じ問い合わせで判定する（設定の `terms` は使わない。別のもので作ると、順に 2 回聞くので 3〜4 秒かかる）。翻訳が Chrome か翻訳専門のモデルなら、用語を決めてから訳す（9.7）
- ふりがなは別の工程にする。今も翻訳とは別の問い合わせで同時に投げているので、分けても遅くならない
- 有料の工程は会社ごとに鍵が要る（`Engines::providers`。工程の選択肢が、会社とモデルの名前を決める）。鍵は自分のキーか、その会社に使えるギフトコード（`Keys::access`。中継のある OpenAI と xAI だけ。Qwen、OpenRouter、TypeSafe AI はギフトコードでは使えず、操作パネルは自分のキーを求める）。工程が使う会社の送り先と鍵は、1 つの一覧（`Accesses`）にして字幕の処理に渡す（会社を足しても、引き回す引数が増えない）。会社ごとの決まり（名前、送り先、ギフトコードで使えるか）は `Provider::info` の 1 つの表、選択肢ごとの会社は工程ごとの `provider` に書く。画面と LP の一覧は、そこから `settings::catalog` が `frontend/src/generated/catalog.ts` に書き出す（`cargo test`。TypeScript に同じ表を手で書かない）。v1.0 でサブスクにして鍵の登録を減らすまでは、鍵が増えることより、選択肢を試せることを優先する。始めるときに足りない鍵を 1 つ選んで `MissingKey { provider }` で止め、操作パネルは足りない鍵を全部出す（`panel.missingKeys`。ギフトコードがあれば足りるとみなす）。ギフトコードが xAI を中継しないと分かったら、始めるときに `GiftUnsupported { provider: xai }` で止める。使わない会社には鍵があっても何も送らない
- 工程の選択肢には、正確さ・速さ・料金を、同じ物差しで同じ順に添える（0.6.0-rc.12。`engines.ts` の `ITEMS`）。前は選択肢ごとに違うことを書き（「Grok の約 7 分の 1」「100 万トークンで入力 $0.1」）、基準のモデルを知らない人には比べられなかった。物差しは工程ごとに 1 つで、欄の説明に書く（`scale`）
  - 正確さ: 文字起こしは、正しく書けた文字の割合（100 − 文字誤り率。録音 2 本の幅。3 章）。用語は、63 文のテストの正解の割合（9.7。辞書の一致は 63 文中 27）。翻訳は、用語の英語名が訳文に残った割合（英語 / ロシア語。9.7。Terra は、用語と同時に聞いた結果のうち用語が合っていた 186 回で数えて 185 / 184）。ふりがなは、難しい 60 文の正解の割合（6.4）
  - 速さ: 測った応答の中央値か幅。Gemini Nano の用語は 1 問 0.37 秒（276 問の中央値）
  - 料金: 話し続けた 1 時間の目安を円で書く（1 ドル 150 円）。文字起こしは 1 分の単価 × 60。用語・翻訳・ふりがなは、測ったときの料金を 1 時間 600 発話に直した概算（翻訳は 1 言語あたり。ふりがなは難しい文で測ったので、ふだんより高めに出る）
  - 測っていないものは書かない（途中の字幕の速さは、選択肢ごとに測り方が違う。Chrome の確定の速さ、Haas-ja の途中の字幕の正確さ）。使った分は「料金」タブの使い道ごとの額で見てもらう
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
- 話す言語が日本語でなくても Chrome は認識できるので、どちらのモードも使える。形態素解析（6.4）は日本語だけなので、ほかの言語の文は読みと用語を付けずに流す
- 翻訳先は Translator が対応する言語すべて。対応しない組み合わせは省いて、操作パネルに知らせる。モデルは Chrome が持つ（初回に Chrome がダウンロードし、進み具合を出す）

**途中で変えた判断:** rc.4 までは途中の字幕を PC の中の認識（SenseVoice）で作り、翻訳を Bergamot で作った。無料のモードの精度が競合の Chrome の認識より低いと使ってもらえず、翻訳先も 4 言語に限られ、別プログラムのダウンロードも要った。そこでどちらも Chrome に任せ、補助プログラム（`jimaku-local`）をやめた。形態素解析は core に組み込む。音声が Google に送られることは、操作パネル、プライバシーポリシー、LP に書く。ふつうのタブで開く形も試したが、フラグが効かないので、専用のプロファイルの起動に戻した。

| 用途 | 採用 | 理由 |
|---|---|---|
| ふりがな、用語の判定 | vibrato + 話し言葉の UniDic（6.4） | 辞書引きなので速く、API も要らない。文脈で決まる読みや造語は誤るので、`standard` の確定の字幕は LLM で読む。辞書はアプリに組み込まれる（無圧縮で約 46MB。Lindera に圧縮の設定はない） |

**速い用語の判定（`terms.rs`）:** デッドロックモードがオンなら、LLM を使わずに字幕の中の用語を探す。規則は 1 つで、**書き方か読みが公式名と同じなら用語にする**。手で保守する表（別名、聞き違いの書き方、除外する言葉）は持たない。辞書から用語の表（`terms::Term`。公式名ごとに、参照、読み、英語名）を作って探す。辞書はあとから届いたり、LLM の読みが増えて作り直されたりするので、変わるたびに作り直す。

1. 書き方の一致: 形態素の区切りに沿って公式名を探す。同じ位置からは長いものを取る
2. 読みの一致: 形態素の読みをつなげたものを、公式名の読みと比べる（呪いの異物 → のろいのいぶつ → 呪いの遺物）。4 文字より短い読みは比べない

読みの長さの下限だけは残す。外すと公式名と違う書き方の言葉が用語になる。「皆」がミナ、「見な」がミナ、「山と」がヤマトになり、訳文の名前も置き換わる（3 文字まで下げても「山と」が残る）。漢字を含む公式名の読みはどれも 4 文字以上なので、下限が外すのは「へいず」のようなかな書きの短い名前だけ。

漢字を含む公式名の読みは、LLM の読みのキャッシュ（8.2）があればそれを、なければ読みの辞書（6.4）が名前を読んだ結果を使う。手で保守する読みは持たない。辞書は固有の名前を読み誤る（人狼 → ひとおおかみ、殺魂剤 → やたましいざい）が、読みの一致で見つからないだけで、書き方の一致は効く。誤った読みは普通の言葉の読みと重ならない。正しい読みの同音語（奇襲と紀州、近接と緊切）は用語になることがあるが、例外の表は足さない。

あいまいな一致はしない。文脈を見ないので、結果は次のようになる。

- 公式名と同じふつうの言葉も用語になる（ポケット、ラッシュ、セブン、シルバーなど）。アイテムと同じ言葉（コンデンサー、サイレンサーなど）は `chrome` ならカードも出る
- 略称（カクマガ）と、読みが公式名と違う聞き違い（角磨が）は用語にならない。`chrome` ではアイコンが付かない
- カタカナの複合語（ヘイズウルト）は 1 つの形態素になるので、用語にならず、翻訳の前の置き換えも効かない。カーストの中のカースのように、文字の並びだけでは別の言葉と見分けられない
- どちらも `standard` と `openai` では LLM が文脈から判定するので、確定で直る。略称と言い間違いは LLM に任せる（9.1）

見つけた用語は `Token::Entity` にする（原文は話した言葉のまま）。翻訳の前に英語名へ置き換え、訳文に残った英語名も用語にする。同じ名前のスキルとアイテムはアイテムにする。`chrome` のアイテム説明はこの確定の字幕から出る。`standard` のアイテム説明は OpenAI の結果（`enriched`）を待つ。途中の字幕は揺れるので、カードを出さない。

途中の字幕は認識が進むたびに文が変わるので、用語のアイコンは文に合わせて付いたり消えたりする。`GrowingText` は用語の全体が出てからアイコンを付ける。OpenAI の確定の文には、受け取った時点で読みと用語を付けて、文とトークンを同時に入れ替える。`enriched` が来たら置き換える。途中から付いていた用語とふりがなには、`enriched` の演出を重ねない。途中の字幕になく、LLM だけが見つけた用語には、原文でも訳文でも演出を付ける（`tokens` と比べる）。

### 9.7 用語と翻訳を分ける（`decide.rs`、`mt.rs`、`assets/aliases.rs`）

LLM に用語と翻訳を 1 回で聞く形（9.1）は、いちばん正確だが、遅くて高い（Terra で中央値 2.5 秒。料金の大半は、毎回渡す辞書の一覧）。安いモデルにすると、用語の判定が大きく落ちる。そこで、用語と翻訳を別々の工程として選べるようにした。LLM の形は残す（OpenAI の鍵だけで最高精度が出る。分けた形を実際の配信で確かめるまでは消さない）。

**用語: PC の中で候補を出し、Decisions API に確かめさせる（`decide.rs`）。** OpenAI の Decisions API（`POST /v1/decisions`、モデルは `gpt-6-luna` だけ）は、文を書かずに選択肢から選ぶだけなので、応答が約 0.2 秒で、出力は課金されない。ただ文の中から用語を探すことはできないので、候補は PC の中で出す。

- 候補: 辞書の名前と別名の一致（長い名前を先に）、どれとも一致しないカタカナの言葉に名前の似た用語（字の 3 分の 2 以上、2 字以上が順に名前に入っているものを 5 つまで。略称「エコシャ」、名前の一部「カート」、聞き違い「リコセ」「ページ」。「ヴァ」と「バ」は同じに扱う）、「ウルト」と番号（文と前の発話に出たヒーローのスキルから。ヒーローが分からなければ候補にしない）
- 問い: 候補ごとに 1 つの選択の問い（その言葉が指しうる用語と「どれでもない」）。1 発話の問いを 1 回で送る。候補は多めに出し、ふつうの言葉かどうか・どのスキルかは文ごとに選ばせる
- 名前が似ているだけの言葉が、同じ文でもう呼ばれた用語を指したら、用語にしない（「インファーナスのウルトでスタン入った」の「スタン」は、ウルトの名前「スタンバースト」の一部だが効果の話。訳文に同じ名前が 2 度出てしまう）
- 断られたら（鍵やギフトコード）字幕を止め、ほかの失敗（3 秒のタイムアウト、古い Worker の 404）は PC の中の辞書の一致で続ける。だから Decisions を選んでも、PC の中の一致は付けておく（`Phase::local_terms`）
- 候補の出し方がかなに頼るので、話す言語が日本語のときだけ（ほかの言語では用語を付けない）

**別名（`assets/aliases.rs`）:** 略称や英語名のカタカナは、辞書の読み（8.2）と同じく LLM（`models.readings`）に一度だけ作らせて、名前ごとに `aliases-llm.ja.json` に置く。人の手では書かない（パッチで用語が増えたら、次の起動で足りない分だけ聞く）。作るのは用語を Decisions で確かめるときだけ。別名は候補を出すだけなので、多少外れていてもよい（確かめるのは Decisions）。別名だけでは足りず（LLM は「カース」や「エコシャ」を出さなかった）、名前の似た言葉を候補に足して初めて揃った

**翻訳: 翻訳専門のモデル（`mt.rs`）。** Qwen-MT-Flash（QwenCloud シンガポールの OpenAI 互換の口、`translation_options`）に、用語を `⟦英語名⟧` に置き換えた文を渡す。翻訳先ごとに 1 回ずつ同時に聞く（1 回 0.2〜0.3 秒）。

- 分野の指定（`domains`）で「括弧の中はゲームの用語で、そのまま残す」と伝える。これがないと括弧を外すことが多い（実測）。それでも外したり引用符に替えたりするので、訳文に残った英語名は括弧がなくても用語に戻す（Chrome の翻訳と同じ考え方）。名前まで訳された用語は、ふつうの文字のまま出る
- 言語コードは `stt` の 2 文字で、簡体字は `zh`、繁体字は `zh_tw`、ノルウェー語は `nb`（`no` は断られる）
- 回数の上限（429）は 2 度まで待って送り直す。上限の値は公開されておらず、続けて送ると 1 分に 100 回ほどで断られた。1 発話につき翻訳先の数だけ送るので、翻訳先が多く話し続ける配信では遅れることがある
- 鍵は地域ごとに別で、シンガポール（国際）の鍵だけが使える（ほかの地域の鍵は 401）。ギフトコードでは使えない（Worker に中継がない。`GiftUnsupported` で止め、操作パネルは自分のキーを求める）
- 単価（100 万トークンで入力 $0.16・出力 $0.49）は料金の比較サイトの値で、公式のページでは確かめていない
- 翻訳が Chrome のときは、決めた用語を英語名に置き換えて Chrome のページに渡す（`finish::translatable`。用語を PC の中の一致で決めるなら今までどおりすぐ頼み、Decisions なら用語が決まってから頼む）

**測った結果**（2026-11、`crates/core/examples/split_eval.rs` と `terms_eval.rs`。文と正解は人が書いた 63 文 `experiments/data/terms_cases.json`、各 3 回で 189 回。候補の出し方を直すときに見ていない文。訳は英語とロシア語）:

| 形 | 用語の正解 | 応答（中央値） | 189 回の料金 |
|---|---|---|---|
| Terra に用語と翻訳を同時（今の最高精度） | 186（98%） | 2.5 秒 | $0.18 |
| Luna に同時（ライト） | 142（75%） | 2.2 秒 | $0.027 |
| 用語だけを一覧で返させる: Terra / Luna | 185 / 150 | 1.8 / 1.6 秒 | $0.12 / $0.022 |
| PC の中の候補 + Decisions（用語だけ） | 174〜176（92〜93%） | 0.22〜0.27 秒 | $0.0075 |
| PC の中の候補 + Jev（用語だけ） | 172（91%） | 0.17 秒 | $0.0046 |
| **アプリの処理をつなげたもの（Decisions → Qwen-MT-Flash。訳まで）** | 177（94%） | 0.55 秒（遅いとき 0.85 秒） | 約 $0.02 |

| 翻訳だけ（正解の用語を `⟦英語名⟧` にした文） | 用語が崩れなかった回数 | 応答（中央値） | 189 回の料金 |
|---|---|---|---|
| Qwen-MT-Flash | 168 / 179 | 0.25 秒 | $0.011 |
| DeepL（`ignore_tags`。無料プラン） | 186 / 189 | 0.60 秒 | 約 $0.1〜0.2（API Pro の単価で） |
| GPT-6 Luna | 178 / 183 | 3.0 秒 | $0.011 |

- つなげたものは、発話の間を 2.5 秒空けて送った（続けて送ると Qwen の回数の上限に当たり、63 回中 19 回で英語の訳が抜けた）。空ければ 189 回すべて訳が返った。ロシア語では 189 回中 19 回、用語の名前まで訳された（その用語は色もアイコンも付かず、訳された名前の文字で出る）
- TypeSafe AI の Jev（`POST https://api.typesafe.ai/v1/systemone`。選択肢から選ぶだけのモデル）も、同じ候補と問いで測った。精度は Decisions と同じくらいで、0.1 秒速く、料金は約 6 割（単価は 10 億トークン $42、出力は無料。料金のページはなく、解説の記事の値）。外し方は違う（Jev は正式な名前「アンストッパブル」を用語にせず、Decisions は「サイレンス食らった」をアイテムにした）。0.6.0-rc.2 で、用語の選択肢として入れた（`TermsEngine::Jev`。問いの形だけ Jev に合わせる `Asked::systemone`）
- **用語の判定を、Chrome の中の Gemini Nano にさせる（2026-11 に測り、0.6.0-rc.10 で用語の選択肢にした。`TermsEngine::GeminiNano`）:** core が候補と問いを作り（`decide::Asker`。会社の判定モデルに送るものと同じ）、Chrome のページに送る（`ChromeCommand::Decide`）。ページは Prompt API で 1 問ずつ聞き、選ばれた選択肢を返す（`frontend/src/chrome/judge.ts`、`ChromeMsg::Decided`）。答えが 5 秒来ない、Gemini Nano を使えない、ページが閉じているなら、PC の中の辞書の一致のまま。モデルの用意（初回は約 4 GB）は、翻訳のモデルと同じ流れ（クリックが要るので、アプリが代わりに押す）。途中の字幕が Chrome のときだけ選べる（`Engines::sanitized`）。実際のアプリで、ページを通した動きはまだ確かめていない（測ったのは、同じ聞き方を測定用の Chrome に直接送ったもの）。測ったときの記録: Gemini Nano を判定モデルと同じ形の口で待たせて（`experiments/chrome_eval.mjs decider`）、アプリと同じ候補と問いを 1 問ずつ聞いた（`split_eval terms-jev local-gemini-nano`。答えは選択肢の番号）。189 回中 135 が正しい（71%）。応答の中央値は 1 文 0.44 秒（1 問 0.38 秒）。Perplexity Decider V1.2 は 178、OpenAI の Decisions は 175。PC の中の辞書の一致だけだと、同じ 63 文で正解と同じになるのは 27 文（43%。`annotate::tests::local_terms_on_the_cases`）なので、サーバーを使わない Chrome のモードでは、用語の強調が大きく良くなる見込み。GPU をゲームと取り合うこと、使える PC が限られることは、ふりがなと同じ（6.4）
- **PC の中で動く汎用の小さい LLM で、翻訳・用語の判定・ふりがなをまかなえるかを測った（2026-11-13。0.7 に向けて。llama.cpp 0.9.0 の `llama-server` に GGUF を載せ、Apple M5 Pro の GPU で。`experiments/llama_decider_server.py`、`mt_eval_llama.py`。答える前の「考え」は `chat_template_kwargs.enable_thinking = false` で止める）:** 対象は、2 台の PC で配信している人か、GPU に余裕がある人。

  | モデル（どれも Apache 2.0、4 bit） | 大きさ | 用語の判定（189 回） | 翻訳（63 文。縛りなし → 縛りあり） | ふりがな（180 回。辞書だけは 153） |
  |---|---|---|---|---|
  | Gemma 4 E4B（Google の公式の QAT q4_0） | 5.15 GB | 156（83%）・中央値 0.06 秒 | 46 → **63**（英語 63 / ロシア語 63）・1 言語 0.28 秒 | 150（直す 15、壊す 18） |
  | Gemma 4 12B（同） | 6.98 GB | 132（70%）・0.12 秒 | 55 → 62・0.61 秒 | 165（直す 18、壊す 6） |
  | Qwen3.5-4B（bartowski の Q4_K_M） | 3.01 GB | 66（35%）・0.05 秒 | 55 → 58・0.29 秒 | 129（直す 12、壊す 36） |
  | Gemma 4 E2B（Google の公式の QAT q4_0） | 3.35 GB | 138（73%）・0.04 秒 | 47 → 58・0.15 秒 | 129（直す 12、壊す 36） |
  | Qwen3.5-9B（bartowski の Q4_K_M） | 6.17 GB | 63（33%）・0.07 秒 | 42 → 45・0.42〜0.49 秒 | 147（直す 18、壊す 24） |
  | Hy-MT2 7B（unsloth の Q4_K_M。翻訳専用） | 4.62 GB | — | 36 → 60（英語 61 / ロシア語 61）・0.42 秒 | — |

  - 翻訳は、Gemma 4 E4B に llama.cpp の文法（GBNF）で縛りをかけると、63 文すべてで用語の名前が崩れなかった。前に測った PLaMo 2B（59〜60、0.44 秒。ライセンスが独自）より正確で速い。縛りは「`⟦` のあとは、その文の用語の名前しか書けない」「用語のない文では括弧を書けない」「括弧の直後に字を続けられない（`⟦Sticky Bomb⟧s`。`'s` は書ける）」。縛りなしで崩れた 15 文は、どれも所有の `'s` を括弧の中に入れたもの（`⟦Kelvin's⟧`）。訳は、ゲームの話し言葉として自然（「遠くから抜かれた」を Got sniped … from way out there。Qwen-MT-Flash は passed と取り違えた）。ロシア語の質は判断しきれていない
  - 用語の判定は、選択肢に A、B、… の記号を振って、記号 1 文字だけを答えさせた（文法で縛る）。Gemma 4 E4B の 83% は、Chrome の Gemini Nano（71%）より良く、Perplexity Decider Flash のローカル版（86〜89%。1.0 秒、GPU のメモリ 19 GB）に近い。大きい 12B のほうが悪い（「用語ではない」を選びすぎる）。Qwen3.5-4B は使えない（ヒーローの「ケルビン」を、ふつうの言葉と答える）
  - ふりがなの読み分けは、どれも辞書だけより良くならないか、少し良いだけ（12B の 165 でも、クラウドの判定モデルの 171 に届かない）。小さいモデルは音読みを選ぶ（「店」を「てん」、「金」を「きん」、「床」を「とこ」）。ローカルでは、辞書のままにする
  - 小さい E2B は、どの工程でも E4B より下（速さは約 2 倍）。Qwen3.5 は、9B でも用語の判定が 33%（「用語ではない」を選ぶ）。翻訳専用の Hy-MT2 7B は、縛りをかけると 36 から 60 に上がるが、E4B（63、0.28 秒）に届かず、遅い
  - Perplexity Decider Flash の公式の GGUF（Q8_0、5.16 GB、Apache 2.0）は、素の llama-server では動かない（モデルの説明に、専用の読み出し方を実装した実行環境が要るとある）。
  - 判定モデル用の実行環境 Ollaya 0.18.0（Apache 2.0。Jev と同じ形の口を `127.0.0.1:11435` に開く）で、判定用に作られたモデルを同じ 189 回で測った（2026-11-13。`split_eval terms-jev ollaya-モデル名`）。どれも、汎用の Gemma 4 E4B（156・0.06 秒）より良くて速い、とはならなかった

    | | 大きさ | 正解 | 応答（中央値） | 動いた場所 |
    |---|---|---|---|---|
    | Perplexity Decider（`pplx-decider`。ONNX、32 bit） | 9.1 GB | 161（85%） | 2.0 秒 | — |
    | Kev | 9.5 GB | 155（82%） | 2.0 秒 | CPU |
    | JevK5（Q8_0） | 4.5 GB | 144（76%） | 0.23 秒 | GPU |
    | Winnow E4B（Gemma 4 E4B を判定用に鍛えたもの。Q8_0） | 8.0 GB | 141（75%） | 0.21 秒 | GPU |
    | Laya（多言語版は 683 MB） | — | 54（29%） | 0.04 秒 | GPU |

    Decider は 5 点だけ上だが、30 倍以上遅く、大きい。PC の中の用語の判定は、Gemma 4 E4B を llama.cpp で動かす案のまま
  - 見立て: 翻訳と用語の判定は、Gemma 4 E4B の 1 つ（約 5 GB、llama.cpp）でまかなえる。メモリの使用量と、Windows の GPU での速さ、ゲームと同時に動かしたときの FPS は、まだ測っていない

- **Google の Gemini も測ったが、入れない（2026-11-12。Gemini API のキーで直接。`experiments/gemini_decider_server.py`、`mt_eval_gemini.py`、`stt_eval_gemini.py`）:**
  - 判定モデルの Gemini Decision 1（`POST /v1beta/decisions`。問いと答えの形は Jev とほぼ同じで、文を `input` の text で渡す。入力 100 万トークン $0.10、出力は無料）は、同じ候補と問いで 189 回中 168・中央値 0.44 秒・約 $0.006。今の選択肢のどれよりも下で、Perplexity Decider の約 5 倍の料金。ふつうの言葉を用語にする（レム睡眠、ビリーって友達、アイビーリーグ、ストーカー）
  - 翻訳だけ（`split_eval translate` と同じ文と数え方、63 文を 1 回ずつ）の Gemini 3.5 Flash-Lite は、用語が崩れなかったのが 51（英語 55 / ロシア語 56）。Qwen-MT-Flash（93%）と Hy-MT2（85%）より下。応答は、無料枠のキーで直接送ると中央値 8 秒、OpenRouter を通すと約 0.8 秒（無料枠は後回しにされる）。Gemini 3.8 Flash は、OpenRouter でも 6〜7 秒かかる（答える前に考える）
  - 文字起こしの Gemini 3.5 Transcribe（`POST /v1beta/interactions`。音声を base64 で渡し、用語は `transcription_config.custom_vocabulary`。1 分 約 $0.005 で、GPT-Transcribe とほぼ同じ）は、測れていない。無料枠は 1 日 25 回までで、合成音声の 126 本を流しきれなかった（途中の結果も残していなかった）。試しの 1 本では、用語を渡すと「ヴィスカス」を公式名どおりに書いた（ほかのエンジンは「ビスカス」）。判定モデルと文字起こしのモデルは、OpenRouter にはない
- OpenRouter には判定モデルが 36 個あり、Jev と同じ形の口（`POST https://openrouter.ai/api/v1/systemone`）で呼べる。同じ候補と問いで 9 個を測った（189 回中の正解、応答の中央値、189 回の料金）: Nace Drex Max 182・0.43 秒・$0.0040、Microsoft Decision 1 179・0.46 秒・$0.0021、Perplexity Decider V1.2 178・0.47 秒・$0.0012、OpenAI の Decisions 175・0.58 秒・$0.0070、Jev 1.14 171・0.33 秒・$0.0045、Perplexity Decider Flash V1.2 165・0.40 秒・$0.0006、Parallel Decisions Fast（無料。Web を調べる）152・0.67 秒、Nace Drex v1.5 152・0.48 秒、Nace Drex v2.0 143・0.42 秒。同じモデルでも OpenRouter を通すと遅い（OpenAI の Decisions は 0.27 → 0.58 秒、Jev は 0.17 → 0.33 秒）。上位は OpenAI に直接聞くのと同じくらい当たり、Perplexity Decider V1.2 は料金が約 6 分の 1。0.6.0-rc.2 で、上位の 3 つ（Drex Max、Microsoft Decision 1、Perplexity Decider V1.2）を用語の選択肢に、Hy-MT2 を翻訳の選択肢に、MAI-Transcribe 2 を確定の文字起こしの選択肢に入れた（鍵は OpenRouter の 1 つ。料金は答えの `usage.cost` をそのまま数える）。既定は、速い OpenAI の Decisions API のまま
- MAI-Transcribe 2（OpenRouter の `/api/v1/audio/transcriptions`、`experiments/stt_eval_openrouter.py`）の確定の文字起こしは、文字誤り率が sample01 で 3.2%、sample02 で 5.9%（「拡マガ」を「カクマガ」と書く）、応答 0.8〜1.5 秒、1 時間 $0.10。Grok と同じくらいで、用語を渡す欄はない
- Nace に直接聞いた Drex Max（`POST https://console.nace.ai/v1/systemone`）は 178・0.39 秒・$0.0036。OpenRouter を通すより少し速いだけで、OpenAI に直接聞く Decisions（0.22〜0.27 秒）より遅い
- OpenRouter の無料のモデルに回す口（`openrouter/free`。`terms_eval.rs` で用語と翻訳を同時に聞く形）は、字幕には使えない。63 文を 1 回ずつで、用語の正解は 22、失敗 32（半分が 30 秒の待ち時間切れ）、応答は平均 22 秒。どのモデルに当たるかは毎回違う（9 種類）。推論を止める指定を付けると、指定を知らないモデルが断って 12 だった
- PC の中で動くモデルも測った（Apple M5 Pro、メモリ 64 GB）。どれも、モデルと一緒に配られている開発元のコードで動く
  - 翻訳の PLaMo 3 Translate（MLX の 4 bit 版、`experiments/local_translate_eval.py`。用語は用語集の機能で「話した言葉 → `⟦英語名⟧`」と渡す。63 文を 1 回ずつ、英語とロシア語を順に訳す時間）: 2B は用語が崩れなかった文が 59、中央値 0.60 秒（遅いとき 0.84 秒）、メモリ約 2.7 GB。0.8B は 48、0.25 秒、約 1.1 GB で、ヒーローの名前ごと落とすことが多い（「ケルビンのアイスパスで逃げられた」が「I got out on ⟦Ice Path⟧.」）。2B は Qwen-MT-Flash と同じくらい用語を守り、1 言語なら同じくらいの速さ。ライセンスは、個人と非商用は無料、商用は売上 10 億円以下なら登録して無料
  - 判定の Perplexity Decider Flash V1.2（45 億パラメータ、重み約 9 GB。`experiments/local_decider_server.py` で Jev と同じ形の口にして、同じ候補と問いを送る）: GPU（MPS）で 189 回中 162、中央値 1.0 秒（遅いとき 1.4 秒）、GPU のメモリ約 19 GB（開発元のコードは CUDA 以外では 32 bit で読む）。問いを 1 つずつに分けると 168 で、遅いときが 2.1 秒。CPU だけだと中央値 6 秒で、63 回中 48 回が 8 秒の待ち時間切れ。精度は OpenRouter の同じモデル（165）と同じだが、API（0.2〜0.4 秒、189 回で 1 セント未満）より遅く、ゲームと同じ PC の GPU を 19 GB 使うので、入れない
  - PLaMo の 2B は llama.cpp（0.9.0、`llama-server` の `/completion` に同じ形の文を送る。GGUF の Q4_K_M、1.8 GB）でも動き、Windows でも同じ形で使える。用語が崩れなかった文は 59〜60。Metal で中央値 0.44 秒（遅いとき 0.60 秒）、メモリ約 2.3 GB。CPU だけだと 8 スレッドで 1.3 秒（遅いとき 1.9 秒）、4 スレッドで 2.0 秒（遅いとき 2.8 秒）、メモリ約 3.6 GB
  - PLaMo のライセンス（PLaMo Community License、2026-11-09 改定）は、個人と学術の利用を超えるものをすべて商用とし（製品やサービスの開発と提供を含む）、商用は登録と、売上 10 億円以下が条件。モデルを人に渡すときは、ライセンスの写しを渡して守らせ、「Built with PLaMo」と書く。訳文は表示なしで出せる。収益のある配信で使う人は、それぞれ登録が要ると読める
  - 文字起こしの Haas-ja v1.0（6 億パラメータ、Apache 2.0）は、開発元のコードだと Linux と NVIDIA の GPU が要るが、speech.cpp（0.10.1。MIT。Metal、Windows と Linux は Vulkan、CPU）の GGUF 版で動く（`experiments/stt_eval_speechcpp.py`、各 3 回）。文字誤り率は sample01 で 3.2%、sample02 で 4.9%（「拡マガ」を「角マガ」、「遺物」を「異物」）で、Grok と同じくらい。録音 1 本（22 秒と 29 秒）の認識は GPU で 0.07 秒、CPU だけ（4 スレッド）で 1.0〜1.5 秒。Q8_0（0.66 GB）と Q4_K（0.35 GB）で文字誤り率は同じ。辞書の公式名を全部（約 600 件）用語として渡すと、sample02 で話していないアイテムの名前を書き、16.7% に落ちた（用語は絞らないと渡せない）。途中の字幕は作れない（録音を最後まで聞いてから書く）
  - Chrome と同じ文と録音で比べた（`experiments/chrome_eval.mjs`。別のプロファイルの Chrome 157 を CDP で動かし、認識には録音を偽のマイクとして聞かせる）。翻訳（Translator に、アプリと同じく用語を英語名に置き換えた文を渡す。63 文）は、英語名が訳文に残った文が英語で 41、ロシア語で 19（両方残ったのは 19）。PLaMo 2B は 60 と 60。Chrome は名前の大文字と小文字を変え（「napalm」「METAL SKIN」「SEVEN」）、ロシア語では名前を訳す（Metal Skin →「металлической кожи」、Silver →「серебро」）。ゲームの言い回しも外す（「ウルト二連発」→「two worts in a row」）。ただし 2 言語で約 10 ミリ秒と速い。この結果から、訳文の英語名は大文字と小文字を区別せずに探すようにした（`translation.ts` の `findWord` と `mt.rs`。原文で見つけた用語だけを探すので、ふつうの言葉と取り違えにくい）。同じ訳文で数え直すと、英語は 41 → 59。ロシア語は名前ごと訳されるので 19 のまま。認識（各 3 回、確定した文をつなげたもの）の文字誤り率は sample01 で 4.0%（「いやヘイズの」→「や平日の」）、sample02 で 6.9%（「拡マガ」→「カクマガ」「角マガ」、「最終盤」→「最終話」）で、Haas-ja（3.2%、4.9%）のほうが少し良い。録音 2 本だけなので、差は確かとは言えない
- Decisions の形が外したのは、ふつうの言葉を用語にする（ポケットWi-Fi）、候補に出ない呼び方（ビバップの「フック」）、聞き違いのヒーローを用語にしない（インフェルノス）。Luna に辞書の全部を渡す形は、ふつうの言葉（シルバー帯、アポロチョコ、レム睡眠）を 33 回中 13〜21 回用語にしたが、Decisions の形は 3 回
- 用語だけを聞いても、安いモデルの判定は伸びなかった（Luna は 75% → 79%）。伸びたのは、聞き方を「探して区切る」から「候補を確かめる」に変えたとき
- Qwen-MT の Plus と Lite は括弧を外すか名前を訳すことが多く、使えない（2026-11-12 に同じ 63 文で測り直した。189 回のうち用語が崩れなかった回数、英語 / ロシア語、応答の中央値、料金。発話の間は 1.5 秒）: Lite は 54（82 / 54）・0.27 秒・$0.0087 で、英語でも括弧ごと外す（「The Infernus’s Napalm」）。Plus は 127（180 / 133）・0.31 秒・$0.18 で、英語は守るが、ロシア語で括弧の中の名前を訳す（`⟦Инфернус⟧`、`⟦Липкой бомбы⟧`）。料金は Flash の約 15 倍。同じ条件の Flash は 176（186 / 179）・0.25 秒・$0.012 で、3 つの中でいちばん良く、いちばん速い。Qwen3.8-LiveTranslate は声から直接訳すので、用語を先に決められず、確定の字幕とも文がずれる（途中の訳の候補にはなる）
- Tencent の翻訳専門のモデル Hy-MT2（Apache 2.0。OpenRouter の `chat/completions` に、モデルの説明にある指示文で送る。system はなく、設定は説明の勧める値。各 189 回、用語が崩れなかった回数、英語 / ロシア語、応答の中央値、料金）: 30B-A3B は 161（187 / 161）・0.77 秒・$0.0050、7B は 120（148 / 137）・0.72 秒・$0.0048、1.8B は 101（162 / 117）・0.68 秒・$0.0028。Qwen-MT-Flash（168、176 / 171、0.25 秒、$0.011）と比べ、30B-A3B は英語でよく守り、ロシア語で少し落ち、料金は半分、応答は 3 倍（OpenRouter を通した時間）。訳はゲームの話し言葉らしい（「I tanked it with ⟦Ivy⟧’s ⟦Stone Form⟧.」）。小さい 2 つは、ロシア語で括弧の中まで訳し（`⟦Металлическая кожа⟧`）、頼んだ言語と違う中国語や、絵文字、指示文の見出しを書くことがある。原文をそのまま渡して用語を指定（「話した言葉 translates to `⟦英語名⟧`」）で伝える形は、30B-A3B で 162、7B で 143、1.8B で 62 で、大きいモデルでしか効かない
- 従来の翻訳 API は文字数で数え、Qwen-MT-Flash より 6〜15 倍高い（100 万文字で Azure $10、Amazon $15、Google $20。比較サイトの値。189 回は約 6,600 文字）。測っていない
- DeepL は用語をいちばん守るが、Qwen-MT-Flash の 10 倍以上高く、無料プランは回数の上限が厳しい
- 前の 37 文（9.3 の正解は Terra の答え）でも測ったが、候補の出し方をその文を見て直したので、数字は参考（Decisions の形は 99/111）

### 9.8 PC の中の文字起こし（`local.rs`）

確定の文字起こしを、PC の中で動く Haas-ja v1.0（Apache 2.0、日本語だけ）にできる。無料で、音声は PC から出ない。精度は Grok と同じくらいで、Chrome より少し良い（9.7 の実測。録音 2 本だけ）。途中の字幕は作れない（録音を最後まで聞いてから書くモデル）ので、途中の字幕は今までどおり Chrome か GPT-Live-Transcribe。

- **動かし方:** speech.cpp（MIT。Metal、Windows と Linux は Vulkan、CPU）を別のプロセスで立て（`speech serve haas-ja-v1.0:q4_k --port N`）、OpenAI と同じ形の口（`/v1/audio/transcriptions`）に発話の WAV を送る。アプリに組み込まず別のプロセスにするのは、C++ のビルドをアプリのビルドに入れずに済み、落ちてもアプリを巻き込まないため。プロセスは字幕を止めると終わる（`kill_on_drop`）
- **入れ方:** 初めて使うときに、GitHub のリリースの zip（版は `local.rs` に固定。Mac は 6 MB、Windows は 30 MB）を取り、SHA-256 を確かめてからデータフォルダの `speech/` に置く。モデル（Q4_K、約 0.35 GB。Q8_0 と文字誤り率が変わらなかった）は speech.cpp が初めての起動で Hugging Face から取る（版は speech.cpp が決めている。`SPEECH_MODEL_DIR` で同じフォルダに置かせる）。待っている間は、操作パネルに「準備しています」と出す（`ServerMsg::TranscriptionStatus` の `preparing`）。M5 Pro で、初回は約 55 秒、22 秒の録音の認識は 0.07 秒
- **途中の字幕（`stt/local.rs`）:** 途中の字幕にも選べる（0.6.0-rc.5）。Haas-ja は最後まで聞いてから書くモデルなので、話している発話を、始まりから今までまとめて 0.3 秒おきに認識し直し、そのたびに発話の文字を置き換える（`Delta::whole`）。途中の字幕と確定は、同じ 1 つのプロセスを使う。録音をアプリの区切りで 3 つの発話にして流すと、文字は 0.3〜0.7 秒おきに進んだ（M5 Pro の GPU。`cargo test -p overlays-core haas_ja_writes -- --ignored --nocapture`）。言い出しは書き直しで揺れる（「いいね。」→「でね、僕の」）。speech.cpp の Realtime の口（OpenAI と同じ形）も試したが、2 回続けて同じだった先頭しか返さないので、言い出しが揺れる発話では 7 秒のあいだ 2 文字しか出なかった。CPU だけのときと、ゲーム中の負荷は測っていない
- **用語は渡さない:** 辞書の公式名を全部渡すと、話していない用語を書いた（9.7）
- **動かない PC:** Intel の Mac（配布物がない）と、日本語以外の話す言語。始めるときにエラーで止める
- **確かめていないこと:** Windows の実機（配布物の SHA-256 と中身は確かめたが、動かしていない）と、ゲームが GPU を使っている最中の速さ。rc.4 まで使っていた別プログラム（SenseVoice）は、Chrome より精度が低く、ダウンロードも要るのでやめた（9.6）。今回は精度が Chrome と同じか少し良く、選んだ人だけがダウンロードする
- **確かめ方:** `cargo test -p overlays-core haas_ja -- --ignored --nocapture`（本物をダウンロードして、録音を文字にする）

### 9.9 PC の中の LLM（`llama.rs`）

翻訳と用語の判定を、PC の中で動く Gemma 4 E4B（Apache 2.0）にできる（0.7.0。`TranslationEngine::Gemma`、`TermsEngine::Gemma`）。無料で、字幕の文字は PC から出ない。配信用の PC が別にある人や、GPU に余裕のある人向け（9.7 の実測と見立て）。文字起こしを Haas-ja（9.8）にすれば、Google と会社のどこにも音声と文字を送らずに、字幕と翻訳を作れる。ふりがなは、辞書より悪かったので足していない（9.7）。

**ローカルのモード（0.7.0-rc.2。`Engines::LOCAL`）:** 途中の字幕は Haas-ja、確定は Whisper large-v3-turbo、用語と翻訳は Gemma 4 E4B、ふりがなは辞書。音声も字幕の文字も PC から出さない（辞書とモデルのダウンロード、ランクの取得は別）。0.6.0-rc.11 でモードから外した「ローカル」（途中も確定も Haas-ja）は、カタカナの用語に弱く、用語と翻訳が外に出るので名前が誤解を招いた。今回は、確定の Whisper にヒーローの名前を渡せて（207 個中 194。Haas-ja は 177）、用語と翻訳も PC の中なので、名前どおりになる。途中の字幕を Whisper に統一すると、プロセスは 1 つ減るが、話している間の半分は GPU が動く（Haas-ja は 4%。9.10）ので、途中の字幕は Haas-ja にした。Haas-ja は日本語だけなので、ほかの言語では途中の字幕を出さず、speech.cpp も立てない（`Settings::plan`、`Settings::uses_speech`）。Windows の実機で、速さと GPU のメモリ、ゲーム中の FPS を確かめてから、正式版に残すかを決める。**「その他のモード」（rc.2 のあと。`ModePicker.svelte`）:** モードの一覧は、迷わず選ぶための場所で、初めから見えるのは、キーなしで無料で始められる 3 つ（「おすすめ」「Chrome」「Chrome+」）。初めは「最高精度」を見せて「Chrome+」をその他に入れていたが、入れ替えた: 最高精度は OpenAI のキーが要り、月 約 8,100 円かかるのに、「おすすめ」との差は数ポイントで、選ぶ人が限られる。Chrome+ は、Chrome の人がサーバーを使わないまま用語を良くできる次の一歩。見える 3 つが全部キー不要になるので、「キー不要」の札はやめた（キーが要るのは、最高精度の札にだけ書く）。Chrome+ は、アプリのページを通した動きと、ゲーム中の FPS を、正式版までに実機で確かめる。選ぶ人が限られるモードは、「その他のモード」を開くと出る（`MODES` の `more`。LP の表にも出さない。その中のモードを使っている間は、開いたまま）。入れているのは 3 つ。「Chrome+」（`Engines::CHROME_PLUS`）は、Chrome のモードの用語だけを、Chrome の中の Gemini Nano に確かめさせる（辞書の一致 43% → 71%。初回に Chrome が約 4 GB を取り、判定のたびに GPU を使う。既定にはせず、選びたい人が選ぶ。名前は、Chrome の上位版だと分かるように）。**モードの説明は札で出す（`MODES` の `traits`）:** 名前の横に、料金（か「ベータ期間中は無料」）と、選ぶ理由を 2 つまで、短い札で並べる（「無料」「使用メモリ最小」「GPU を使う」など）。下の文は、何で作るかだけにする。札は、PC の負担を書く: ローカルは「高スペック PC 向け」（「GPU を使う」「CPU を使う」は仕組みの言葉で、自分の PC で選んでいいかが読み取りにくい。どのくらいかは、下の文に GPU のメモリの量で書く）、Chrome+ は「GPU を使う」（判定のときに一瞬使うだけなので、高スペック向けとは言わない。「用語に強い」のような得の札は付けない: 「おすすめ」より強いと読める。何が良くなるかは、名前の「+」と下の文で伝わる）、クラウドは「使用メモリ最小」。「Chrome を使わない」は札にしない（クラウドだけに付けると、ローカルは使うように見える。どちらも使わないので、下の文に書く）。残りの 2 つは、名前を対にした（全部サーバーで作る「クラウド」と、全部 PC の中で作る「ローカル」。「おすすめ（Chrome なし）」という名前は、おすすめの派生に見えるのでやめた）。「クラウド」の得は、Chrome を開かないので PC の負担がいちばん小さいこと（アプリだけなら、メモリは約 20 MB。ユーザーの実機での値）で、説明に書く。「最低負荷」「軽量」という名前も考えたが、名前は仕組みのままにした。その他のモードの中なので、説明は短くする（細かい注意は、工程ごとの選択肢に書いてある）: 「クラウド」（`Engines::CLOUD`。「おすすめ」の途中の字幕も Qwen で作る。Chrome のウィンドウを開かず、音声を Google に送らない。ベータのコードで使える。上限に届くと Chrome の作り方に切り替わるので、そのときは Chrome が開く）と、「ローカル」。工程ごとの選択を自分で触らなくても、1 回で選べるようにしておく。**カスタムは一覧に入れない（rc.4）:** 自分で組んだ組み合わせは、選ぶものではなく今の状態（下の工程ごとの選択の結果）なので、モードのラジオボタンの列から外した。使っている間は、一覧の下に「カスタム: 下の工程ごとに選んだ組み合わせを使っています」と出す（どのモードも選ばれていない）。モードを使っていて、前のカスタムを覚えている間は、「前に組んだカスタムに戻す」を出す。**期待を上げすぎない（rc.2 のあと）:** Chrome で満足している人が選んで、がっかりしないよう、モードの説明に先に書く。選ぶ理由は「外に送らない」ことで、精度ではない: 文字起こしの正確さは Chrome と同じくらい（録音 2 本で、Chrome 93〜96%、Whisper turbo 94〜98%）。用語（辞書の一致 43% → Gemma 83%）と翻訳は Chrome より良いが、GPU を使わない「おすすめ」のほうが上（用語 94%）。ゲームと同じ GPU を使うので、余裕がないと FPS が落ちたり字幕が遅れたりする

- **動かし方:** llama.cpp（MIT）の `llama-server` を別のプロセスで立て（`--model … --n-gpu-layers 99 --ctx-size 8192 --parallel 4 --no-webui`）、OpenAI と同じ形の口（`/v1/chat/completions`）に頼む。別のプロセスにする理由は speech.cpp と同じ（9.8）。立てて待つ部分は、speech.cpp と共通（`sidecar.rs`）。翻訳と用語の両方に選んでも、プロセスとモデルは 1 つ。頼むことがなければ立てない（翻訳先がない、Deadlock モードでない。`Settings::uses_llama`）。途中の字幕と確定が Chrome でも使える
- **翻訳（`mt.rs`）:** 用語を `⟦英語名⟧` に置き換えた文を、翻訳先ごとに同時に頼む（会社の翻訳専門のモデルと同じ流れ）。答えは文法（llama.cpp の GBNF）で縛る: 括弧の中には、その文の用語の名前しか書けない。用語のない文では括弧を書けない。括弧の直後に字を続けられない（`⟦Sticky Bomb⟧s` を防ぐ）。縛らないと 63 文中 8 文で名前が崩れ、縛ると 0（9.7）
- **用語の判定（`decide.rs`）:** 候補と問いは、会社の判定モデルに送るものと同じ（`Asker`）。問いを 1 つずつ同時に頼み、選択肢に A、B、… を振って、記号 1 文字だけを答えさせる（文法で縛る）。1 つでも答えが返らなければ、その文は辞書の一致のまま
- **考えさせない:** `chat_template_kwargs.enable_thinking = false`。考えを本文に書くと、字幕に間に合わない
- **入れ方:** 初めて使うときに、llama.cpp の配布物（GitHub のリリース。ビルドは `llama.rs` に固定。Mac は tar.gz 17 MB、Windows は Vulkan 版の zip 38 MB）と、モデル（Google の公式の GGUF `gemma-4-E4B_q4_0-it.gguf`、5.15 GB。Hugging Face の、版を固定した URL）を取り、どちらも SHA-256 を確かめてからデータフォルダの `llama/` に置く。モデルは `.part` のファイルに書きためて、止まったら次は続きから取る（`download::model`）。待っている間は、操作パネルに「準備しています」と出す（9.8 と同じ）。進み具合はまだ出していない
- **アプリと同じ処理で測った結果:** 用語の正解は 189 回中 156、訳文の用語は 189 回すべて崩れず、判定と 2 言語の翻訳を合わせて中央値 0.39 秒（遅いとき 0.66 秒。M5 Pro。`SPLIT_EVAL_LLAMA=… split_eval pipeline gemma+gemma`）
- **動かない PC:** Intel の Mac（GPU を使えないので配らない）。始めるときにエラーで止める
- **残ること:** アプリが強制終了されると、llama.cpp のプロセスが残る（speech.cpp も同じ。ふつうに終わるときと、字幕を止めたときは終わらせる）。途中でプロセスが落ちたら、立て直さない（訳と判定が付かなくなる。字幕を始め直すと戻る）
- **確かめていないこと:** 配布物を実際に取って動かす流れ（開発の PC では、同じビルドの Homebrew の `llama-server` で確かめた。配布物の SHA-256 と中身の場所は確かめた）。Windows の実機での速さと GPU のメモリ、ゲームが GPU を使っている最中の速さ。GPU のメモリが足りない PC での動き
- **確かめ方:** `LLAMA_TEST_DIR=… cargo test -p overlays-core local_llm -- --ignored --nocapture`

### 9.10 PC の中の Whisper（`whisper.rs`）

確定の文字起こしを、PC の中で動く Whisper にできる（0.7.0。`FinalSource::WhisperTurbo`、`WhisperLarge`）。無料で、音声は PC から出ない。Haas-ja（9.8）との違いは、用語を少し教えられること: ヒーローの名前（話す言語で、44 個）を `prompt` で渡すと、合成音声の 207 個の用語のうち turbo は 194、large-v3 は 203 を書けた（Haas-ja は 177、会社で一番良かった GPT は 191。3 章）。そのかわり、Haas-ja より 10〜25 倍遅く、GPU を使う。どの言語でも書ける。

- **モデル:** large-v3-turbo の q5_0（0.57 GB）と、large-v3 の q5_0（1.08 GB）。小さくしても、元と精度も速さも変わらなかった（turbo: 元 194 / q5_0 194。large-v3: 元 202 / q5_0 203）。録音 2 本の文字誤り率（ヒーローの名前を渡して）は、turbo が 1.6% と 5.9%、large-v3 が 1.6% と 4.9%
- **渡す用語はヒーローの名前だけ:** 辞書の公式名を全部渡すと効かず（Whisper が読むのは終わりの 224 トークンまで）、応答が 2.4 秒以上に延びる。日本語向けに鍛え直した kotoba-whisper v2 は、何も渡さなくても 164 で、用語を渡すと何も書かなくなる
- **動かし方:** whisper.cpp（MIT）の `whisper-server` を別のプロセスで立て（`--inference-path /v1/audio/transcriptions`）、発話の WAV と、言語、ヒーローの名前を送る。立てて待つ部分は、ほかと共通（`sidecar.rs`）
- **配布物は自分たちで作る（`.github/workflows/whisper.yml`）:** whisper.cpp の公式の配布物には、Mac の実行ファイルと、Windows の GPU 全般（Vulkan）用がない（Windows は CPU 用と、285〜685 MB の NVIDIA 専用だけ）。コミットを固定してビルドし（Mac は Metal、Windows は Vulkan、Linux は CPU。ほかの DLL が要らないように固めて、実行ファイル 1 つ）、zip と SHA-256 を、このリポジトリのリリース `whisper-v版` に載せる。「最新」にはしない（アプリの自動更新は、最新のリリースを見る）。アプリは、その zip とモデル（Hugging Face の、版を固定した URL）を取り、SHA-256 を確かめてからデータフォルダの `whisper/` に置く。faster-whisper は、GPU が NVIDIA だけで、Python ごと配ることになるので使わない（速さは測っていない）
- **途中の字幕（`LiveSource::Whisper`、`stt/local.rs`）:** Haas-ja と同じやり方で出せる（話している発話を、始まりから今までまとめて認識し直す。前の認識が終わってから 0.3 秒あけて次を頼む）。途中の字幕と確定は、同じ 1 つのプロセスとモデルを使う（確定が Whisper ならそのモデル、ほかなら turbo。`Engines::whisper`）。ヒーローの名前を渡せて、日本語以外でも出せるが、GPU の負担が大きい。合成音声 42 発話を、実時間で話しているものとして数えた（`experiments/stt_live_eval_local.py`。M5 Pro）:

  | | 1 回の認識（中央値） | 文字が進む間隔 | 話している時間のうち、認識していた割合 | 途中の文字の違い |
  |---|---|---|---|---|
  | Haas-ja（GPU） | 0.013 秒 | 0.31 秒 | 4% | 11.0% |
  | Haas-ja（CPU） | 0.056 秒 | 0.36 秒 | 16%（CPU） | 12.6% |
  | Whisper large-v3-turbo q5_0 ＋ ヒーローの名前 | 0.22 秒 | 0.52 秒 | 49% | 11.8% |
  | Whisper large-v3 q5_0 ＋ ヒーローの名前 | 0.39 秒 | 0.69 秒 | 73% | 13.0% |

  「途中の文字の違い」は、途中の文字を、同じモデルの確定の書き出しのうちいちばん近いものと比べた文字誤り率（言いかけの揺れ）。モデルによる差は小さい。確定より長く書いた（話していない続きを足した）のは、turbo が 299 回中 2 回、large-v3 が 202 回中 3 回、Haas-ja は 0 回。ローカルの組み合わせの既定は Haas-ja のまま（GPU をゲームに空ける）。Whisper は、日本語以外で途中の字幕を出したい人と、GPU に余裕のある人の選択肢。確定の依頼は、途中の依頼 1 回分（0.2〜0.4 秒）だけ待つことがある（whisper.cpp は 1 つずつ処理する）。アプリを通して動かすと（途中の字幕も確定も turbo、用語と翻訳は Gemma）、合成音声 3 発話で途中の文字が進み、訳文つきの確定まで出た。言い出しの短い音声に、話していない「はい。」を 1 回書いた（すぐ次の認識で置き換わる）
- **確かめたこと:** Mac で、リリースの zip を実際に取り、SHA-256 を確かめて広げ、Metal で合成音声を文字にした（上のテスト）。Mac の実行ファイルは、OS の部品だけを使う。Windows の実行ファイルが要る DLL は、OS のもののほかは `vulkan-1.dll` だけ。全部 PC の中の組み合わせ（途中の字幕 Haas-ja、確定 Whisper、用語と翻訳 Gemma）で、合成音声から訳文つきの字幕まで出た
- **確かめていないこと:** Windows の実機（速さ、GPU のメモリ、ゲーム中の動き。`vulkan-1.dll` は GPU のドライバーが入れるので、ドライバーのない PC では立ち上がらない見込み）。無音や雑音だけの区間を渡したときに、話していない文を書かないか（Whisper の知られた癖。アプリは声のある区間だけを切り出して渡す）
- **確かめ方:** `WHISPER_TEST_DIR=… cargo test -p overlays-core whisper_writes -- --ignored --nocapture`

### 9.11 PC の中の AI に使う GPU（`local.rs` の `Gpu`）

GPU が 2 つ以上ある PC（CPU に内蔵の GPU と、あとから付けた GPU）では、PC の中の AI（speech.cpp、whisper.cpp、llama.cpp）に、どの GPU を使うかを渡す（0.7.0-rc.4）。渡さないと、遅いほうで動く。Windows の実機（内蔵の AMD Radeon と RTX 3070）で、何も渡さないときの動きを一覧から読んだ（rc.3）:

- 一覧は `Vulkan0`（内蔵の Radeon、17 GB）、`Vulkan1`（RTX 3070、8.6 GB）の順。llama.cpp（`--list-devices`）と speech.cpp（`devices`）で、名前も並び順も同じだった（どれも ggml の Vulkan で数える）
- whisper.cpp は 0 番の GPU を使う（＝内蔵）。llama.cpp は、見つけた GPU に層を分けて載せる（内蔵の GPU は、PC のメモリを分け合うので空きが多く見え、多く載る）。speech.cpp は「最初の GPU」
- **メモリの多さでは選べない:** 内蔵の GPU のほうが多く見える。speech.cpp の一覧は、内蔵（`igpu`）とそうでないもの（`gpu`）を分けて出すので、それで選ぶ

**決め方（`local::pick`）:** 設定（`Settings::local_gpu`。GPU の名前）で選んだ GPU があれば、それ。なければ、内蔵でない GPU を優先し、同じ種類ならメモリの多いもの。GPU が 1 つ以下なら、何も渡さない（今までどおり、実行ファイルに任せる。Mac はいつもこちら）。

**渡し方:** 字幕を始めるときに、speech.cpp の `devices` で一覧を取る（数えるためだけに speech.cpp を使うので、Haas-ja を使わない組み合わせでも、PC の中の AI を使うなら speech.cpp を取る。Windows は 30 MB）。3 つとも同じ GPU に: speech.cpp と llama.cpp には `--device Vulkan1`、whisper.cpp には `--device 1`（`Vulkan` の後ろの番号）。llama.cpp は、1 つの GPU だけに載る。

**選ぶ画面（`LocalGpu.svelte`、`GET /api/gpus`）:** 「モード」のタブのいちばん下に、GPU が 2 つ以上あるときだけ出す。「自動（RTX 3070）」と、一覧（内蔵かどうかと、メモリ）。ゲームは外付けの GPU、AI は内蔵の GPU、と分けたい人が選び直せる（内蔵は遅い）。一覧は、speech.cpp を取ってあるときだけ出る（見るだけのためには取らない。PC の中の AI を一度動かすと出る）。変えると、字幕を始め直す。

**確かめたこと:** 実機の一覧の文字を読んで、RTX 3070 を選ぶこと（`local::tests`）。同じ一覧を返す見本を置いた環境で、画面に「自動（NVIDIA GeForce RTX 3070）」と出ること。**確かめていないこと:** 実機で、3 つとも選んだ GPU で動くこと（特に、whisper.cpp の番号が、一覧の番号と同じ GPU を指すこと）と、速さがどれだけ変わるか。RTX 3070 のメモリは 8 GB で、Gemma（約 5 GB）と Whisper と Haas-ja を載せると、ゲームの分が足りなくなるかもしれない

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
- **注意は中身の欄の上に出す**: 選んだモードに要る鍵がない（有料のモードで、その会社の API キーもギフトコードもない。「キーかギフトコードを入れる」と「無料の Chrome モードにする」）、字幕が止まった理由、設定を保存できない
- **無料のモードが基本**: 新しく入れた人の既定は Chrome のモードで、使う人の大半はこのまま。モードの選択肢は Chrome を先頭に置き、説明は、途中の字幕に Chrome を使う 3 つを「高速」（確定まで待つ OpenAI だけのモードより、途中の字幕が速い）、確定と翻訳に GPT を使うスタンダードと OpenAI を「最高精度」にする。スタンダードにおすすめの印を付ける（OpenAI のキー 1 つで済み、精度も一番高い。ライトは安いが、OpenAI と xAI の 2 つのキーが要る）。鍵の話は鍵の要るモードを選んだときだけ出す
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
| API キー | ギフトコード（人からもらったおすそ分けのコード。17.1。入れたら使えるか確かめてから保存し、期限と、使える会社（おすそ分けした人がキーを預けた会社）を出す。自分のキーがあればその会社には自分のキーを使う）。OpenAI の API キーの作り方（登録 → クレジットカードの登録と前払いのチャージ → キーの作成 → 貼り付け。OpenAI を使ったことのない配信者向け。キーが保存されていなければ開いておく）。OpenAI（文字起こし・翻訳・読み）と xAI（ライトなどの Grok の確定の文字起こし）。状態（保存済み・未設定・読めない。読めないときは理由を出す）、キーの作成ページへのリンク、新しいキーの保存・削除。キーそのものは表示しない |
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
- **更新の内容（`CHANGELOG.md`、`WhatsNew.svelte`、`lib/changelog.ts`。0.6.0-rc.11）:** 更新したあとの最初の起動で、何が変わったかをダイアログで出す。GitHub のリリースノートは読まれないので、画面の場所や送り先が変わることをアプリの中で知らせる。使う人向けの文を、リポジトリの `CHANGELOG.md` に版ごとに書き（`## 版` と `- ` の箇条書きだけ。書く場所は 1 つ）、アプリがビルドのときに読み込む。設定に、最後に見た版（`notes_seen`。プレリリースの印は付けない）を持ち、それより新しくて今の版までの内容をまとめて出す（何版か飛ばした人には、見ていない分を全部）。プレリリースは本リリースの版の内容を 1 回だけ出す（rc ごとには出さない）。新しく入れた人には出さない（設定のファイルがなければ、今の版まで見たことにする。はじめかたがある）。この項目ができる前（0.5 まで）から使っている人には、今の版までの内容を出す。「このアプリについて」からも開ける

## 13.7 第三者のライセンス表示（`notices/`、`frontend/scripts/notices.ts`）

- `notices/third-party.txt` は `mise run notices` で作ってコミットする。手で編集しない。読みの辞書（現代話し言葉 UniDic）の表示は別のファイル（`notices/unidic.txt`。利用規約でも読む）
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

**0.6 で非推奨にし、0.7 で取り除く（2026-11-13 に決めた）。** キーなしで良い字幕を試してもらう役目は、ベータの無料の提供（17.2）が引き継いだ。残る使い道は「最高精度を知り合いに試してもらう」くらいで、本番のデータベースで使っていたのは開発者 1 人だけだった（おすそ分けをしている人 1、ギフトコード 2）。置いておくと、サーバーが人の API キーを預かり続ける（漏れたときにいちばん困るもの）。0.6 では、取り除かずに知らせるだけにする: 「おすそ分け」のタブの上に知らせを出し、サイドバーの一番下に移し、ギフトコードの欄を API キーの下に置き、更新の内容に書く。新しいおすそ分けは止めない（使う人がいないので、止めても変わらない）。0.6.0 に削除を入れないのは、ベータのコードが同じ表と中継の一部に乗っていて、切り分けに手間がかかるため。0.7 で、アプリとサーバーから取り除き、預かっているキーを消す。

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
### 17.2 ベータ期間中の無料の提供（`server/src/beta.ts`、`Engines::BETA`）

使う人が数えられる人数のうちは、いちばん安い組み合わせを開発者の費用負担で無料にする（2026-11。v1.0 でサブスクにするまで）。鍵の登録なしで、Chrome だけのモードより良い字幕（確定の文字起こし、用語、翻訳）を試してもらうため。

- **組み合わせ（ベータのモード）:** 途中の字幕は Chrome、確定は Qwen-Audio-3.1-ASR-Flash、用語は Perplexity Decider、翻訳は Hy-MT2、ふりがなは辞書の候補から Perplexity Decider が選ぶ（0.6.0-rc.8 から。6.4。費用は 180 文で $0.0009 で、合計はほぼ変わらない）。話し続けた 1 時間で約 $0.035 の概算（9.7 と 3.4 の実測から。毎日 3 時間で月に約 $3）
- **「おすすめ」への導線（0.6.0-rc.15）:** 既定は Chrome のままにする（音声の送り先が変わるので、選ぶのは本人）。そのぶん、気づけるようにする。Chrome のモードの人には、「マイクと翻訳」のタブのモードの欄に「『おすすめ』のモードにすると、字幕と翻訳がもっと正確になります」と出し、「おすすめにする」で同意のダイアログに進む（「閉じる」で次から出さない。`hybrid_hint_closed`。0.5 のときの「スタンダードにする」の提案と同じ項目）。更新したあとの「更新しました」のダイアログにも、Chrome のモードの人には「おすすめにする」のボタンを出す。同意のダイアログは、どのタブからも出せるように操作パネルの根元に置く（`Control.svelte`）
- **コード:** ベータのコードが要る作り方（「おすすめ」のモードか、中継する会社の工程。自分のキーもコードもないとき）を選ぶと、変える前に同意のダイアログを出す（`BetaDialog.svelte`、`panel.betaOffer`。0.6.0-rc.11）。書くのは、ベータ期間中は無料なこと、1 日の上限があり予告なく止まること、音声と文字が開発者のサーバーを通って外に出ること。Chrome のモードは開発者のサーバーを使わないので、送り先が変わる前に聞く。「有効にする」を押すと、アプリが `POST /v1/beta/codes` でコードを 1 つもらい、ギフトコードとは別の所（資格情報ストアの `beta`）に保存してから、選んだ作り方にする（`panel.acceptBeta` → `POST /api/beta`）。もらえなければ、作り方は前のまま。使う人は何も入力しない。ダイアログには、配信の概要欄などに「字幕: デドロ字幕 https://jimaku.deadlock.jp」と載せてもらうお願いと、コピーのボタンも置く（`CreditCopy.svelte`。「このアプリについて」にも置く）。開発者が料金を負担する代わりの広告のつもりだが、条件にはしない（載せたかは確かめられず、条件にすると頼んだ表示として広告の明示が要りうる）。コードはおすそ分けのコードと同じ表（`codes`。持ち主は `beta`）に置くので、1 日の上限・止める・料金の集計は同じ仕組みで動く。人からもらったギフトコードとは別に持つ（0.6.0-rc.11。`Keys::access` が、会社を中継するコードの種類で選ぶ。OpenAI と xAI はギフトコード、Qwen と OpenRouter はベータのコード）。rc.10 までは同じ所に 1 つだけ置いていたので、ギフトコードを持つ人は「おすすめ」のモードを使えず、ベータのコードを持つ人がギフトコードを入れるとベータのコードが消えた。字幕を始めるときは、使うコードをどれも確かめる（`runtime::gift_accepted`）。rc.10 までにもらったベータのコードは、ギフトコードの所に残る（移さない。プレリリースの人だけで、同意のダイアログからもらい直せる。残ったコードは「API キー」のタブで消せる）
- **途中の字幕の中継（0.6.0-rc.7、`server/src/qwenLive.ts`）:** `/v1/qwen/api-ws/v1/inference`（WebSocket）。アプリは自分のキーのときと同じ形で送るので、アプリの側は変えていない。Worker は開発者のキーで Qwen につなぎ、アプリが送る形（`run-task`、16kHz の PCM のバイナリ、`finish-task`）だけを通す。`run-task` は作り直して送る（モデルは Qwen-Audio-3.1-ASR-Flash-Streaming だけ。ほかのパラメータを足させない）。料金は、つなぐ前に音声 30 秒分の見積もり（1 秒あたり入力 11・出力 3 トークンの倍）を先に足し（上限に届いていたら 403 の `limit`）、音声を 30 秒分受けるごとに、Qwen が知らせる使用量で実際の額に直してから次の分を足す（足せなければ `limit` で切る。使用量が届く前は音声の長さで数える）。実時間より速く送られても、音声の長さで区切るので上限を越えるのは 1 回分まで。開発者のキーが断られたとき（つなぐときの 401 など、つないだあとの `task-failed`）は、キーを後回しにして、利用者には中身を渡さずに 502 か 1011 で切る（アプリは張り直す）。手元の Worker から本物の Qwen に通して、アプリの数えた額と Worker の記録が同じ $0.00028（約 50 秒の録音）になることを確かめた。Workers の WebSocket は音声を Blob で渡すのが既定なので、`binaryType` を `arraybuffer` にする
- **中継:** `/v1/qwen/…/generation`、`/v1/openrouter/systemone`、`/v1/openrouter/chat/completions`。開発者のキー（Worker の秘密の設定 `BETA_QWEN_KEY`、`BETA_OPENROUTER_KEY`）で送る。アプリが送る形と、いちばん安いモデルだけを通す。Qwen と OpenRouter のほかの口（ほかのモデル）は 403（`unsupported`）で、アプリは「このコードでは使えない」と止める。おすそ分けのコードでは Qwen と OpenRouter を中継しない（キーを預ける先は OpenAI と xAI のまま）
- **どのコードがどの会社を中継するか:** アプリは会社ごとに「コードで使えるか」だけを持ち（`ProviderInfo::relay`。OpenAI と xAI はギフトコード、Qwen と OpenRouter はベータのコード）、そのコードが実際に中継するかは、字幕を始めるときに `GET /v1/gift` の `providers` で確かめる（今までと同じ）
- **上限:** 1 つのコードは 1 日 $0.5（約 14 時間分）。ベータ全体で 1 日 $5（届いたら、その日はどのコードも中継しない。悪用されても、ここで止まる）。コードは同じ所（IP アドレスと日付のハッシュ。IPv6 は先頭の 64 ビットにまとめる。IP アドレスそのものは保存しない）に 1 日 3 つ、全体で 1 日 100 まで。値は `beta.ts` の定数。各社の管理画面でも、キーに月の上限を付けておく
- **ベータのコードを、自分のキーより先に使う（0.6.0-rc.16。`Keys::access`）:** Qwen と OpenRouter は、ベータのコードがあればコードで送り、なければ自分のキーで送る。rc.15 までは自分のキーが先で、キーを入れている人は「おすすめ」のモードでも黙って自分に請求されていた（「おすすめは無料」という案内と合わない）。コードで断られたら（1 日の上限など）、その会社すべてに自分のキーがある人は、同じ作り方のまま自分のキーで続ける（`Fallback::OwnKeys`。精度は落ちない。操作パネルに「自分の API キーで続けています。ここからの料金は、自分に請求されます」と出す）。キーがなければ、下のとおり Chrome の作り方で続ける。ギフトコードの会社（OpenAI、xAI）は、今までどおり自分のキーが先（コードの分は、おすそ分けした人に請求されるため）。自分のキーだけがあってコードのない人には、同意のダイアログを出さず、自分のキーで使う（開発者のサーバーを通したくない人が、キーを入れて「おすすめ」の組み合わせを使える）。上の帯の「起動してからの料金」は、自分のキーで送る会社があるときだけ出す（`panel.pays`。core と同じ決まりで見る）
- **上限に届いても、字幕は止めない（0.6.0-rc.13。`Store::fall_back`）:** 配信者にとっては、精度が落ちるより字幕が止まるほうがずっと困る。ベータのコードで断られたら（1 日の上限、止められた、提供が終わった。字幕を始めるときも、途中でも）、エラーで止めずに、工程を Chrome のモードにして続ける。保存してある設定は変えず（`Store::saved`）、動かす側だけが替えたものを読む（`Store::settings`。操作パネルには `SettingsView::running` と `fallback` で伝え、「Chrome の作り方で続けています」と出す。配信の画面には何も出さない）。字幕を止めるか、作り方を選び直すと、次は選んだ作り方で始める（まだ断られるなら、また Chrome で続く）。途中の字幕を Chrome で作っていない組み合わせでは続けない（開いていない Chrome を開いて、選んでいない送り先に音声を送ることになるので、今までどおりエラーで止める）。上限は管理画面から変えられるので（17.3）、アプリの文には時間の数字を書かず、「上限に届いても、字幕は Chrome の作り方で続きます」と書く。実際に上限に当てて確かめてはいない（`store::tests` で、切り替えと戻し方を確かめた）
- **上限の確かめ方:** 誰でも叩けて、開発者の費用になるので、「見てから足す」ではなく「足してから見る」。外に送る前に、かかりそうな額（本文の長さから出す多めの見積もり）を、コードの今日の分と全体の今日の分（`beta_spend`。1 日 1 行）に、上限に届いていないことを条件にした 1 つの文で足す（`reserveBeta`）。応答のあとに実際の額に直す（`settleBeta`。額が分からない応答は見積もりのまま）。同時に送られても、越えるのは 1 回の見積もりまで。コードを出す数も、同じ理由で足してから見る。おすそ分けの中継（17.1）は、知り合いに渡したコードだけが使うので、今までどおり応答のあとに足す
- **外に送るもの:** 届いた本文をそのまま渡さず、アプリが送る項目だけで作り直す（Qwen のパラメータや、問いの中の余分な項目で、数えられない機能を使わせない）
- **外の不調:** 開発者のキーが使えない・残高がないなどは、中身を返さず 502 にする（401 をそのまま返すと、アプリは「コードが使えない」と出してしまう）。回数の上限（429）だけは、アプリが待って送り直せるように伝える
- **やめ方:** 秘密の設定のキーを消すと、コードは出なくなり（503）、中継は `unsupported` になる
- **確かめ方:** Worker のテスト（外への通信は差し替える）と、`BETA_TEST_API=http://127.0.0.1:8799/v1 cargo test -p overlays-core beta_requests -- --ignored`（手元の Worker に、アプリの部品が実際に送る要求を通す。偽のキーなので「外の不調」の 502 が返る。400 なら形が合っていない）
- 利用者の音声と文字を開発者のキーで送ることになるので、プライバシーポリシーと利用規約に書く（`web/src/pages/privacy.astro`、`terms.astro`）

### 17.3 管理画面（`server/src/admin.ts`、`adminPage.ts`、`access.ts`、`adminWorker.ts`、`wrangler.admin.jsonc`）

ベータを開発者の費用負担で続けるので、費用とコードを見て、止められる所が要る（2026-11）。API とは別の Worker（`deadlock-jimaku-admin`）にする。API は誰でも叩けるので、Worker ごと Access の後ろに置く管理画面とは、同じ Worker にできない（Cloudflare の管理画面の Workers & Pages → Worker → Access で「All traffic」を守る形にすると、その Worker のどのホストも守られる）。API の Worker に管理の口を持たないので、API の側の攻められる所も増えない。データベースは同じものを使う。

- **入口は Cloudflare Access:** ログインの仕組みは自分で書かない（そこが弱点になる）。Access が前に立ち、通した人の JWT を `Cf-Access-Jwt-Assertion` に付けてくる。Worker は、その JWT が Cloudflare の鍵の署名で、この管理画面あて（`ACCESS_AUD`）で、期限内かを確かめる（`access.ts`）。`TEAM_DOMAIN` と `POLICY_AUD`（Access を有効にしたときに Cloudflare が出す名前のまま）を入れるまで、管理の口は誰も通さない（503）
- **ページ:** 応答の CSP で、ページに書いたスクリプト（そのハッシュ）だけを動かし、枠の中に入れさせず、残させない（`Cache-Control: no-store`）
- **ほかのサイトからの書き換えを受けない:** Access のクッキーは、ほかのサイトからの要求にも付く。書き換える要求は、ページの JavaScript が付けるヘッダー（`X-Admin`）があるものだけを受ける
- **見るもの:** 今日と今月の費用（上限との比）、今日出したコードの数、今日使ったコードの数、コードごとの使用量（今日よく使った順に 100）
- **操作:** コードを止める・戻す、ベータを閉じる・開ける（`beta_settings` の `closed`。閉じると、コードを出さず、中継もしない）、開発者のキーを足す・止める・消す
- **開発者のキー（`beta_keys`）:** 会社ごとに複数置ける。暗号化は API の Worker に頼む（`worker.ts` の `Vault`。サービスバインディングの口で、外からは呼べない）。暗号化の鍵（`SHARE_KEY`）は API の Worker だけが持ち、復号する口は作らないので、管理画面の Worker はキーを読めない。暗号化して保存し、画面には終わりの 4 文字だけを出す。中継は、有効なキーを「最近断られていない順」に使い、断られたら（401・402・429。403 は本文が断られたときにも返るので入れない）次のキーに回して、断られたことを覚える。キーを読めないとき（暗号化の鍵を替えた、行が壊れた）も同じ（`betaRelayKeys`、`relayed`）。同じ条件のキーは毎回並べ替えて、負荷を分ける。重み付けや、会社をまたいだ振り分けはしない（要るようになってから）。Worker の秘密の設定のキー（`BETA_QWEN_KEY`、`BETA_OPENROUTER_KEY`）は、管理画面のキーがどれも断られたときの最後の頼みとして残す
- **上限の設定（`beta_settings` の `dailyUsd` と `codeDailyUsd`。`betaLimits`）:** ベータ全体の 1 日の上限と、1 つのコードの 1 日の上限を、管理画面から変える（`PUT /api/settings`。正の数で $1000 まで。桁の打ち間違いをここで止める）。使う人が増えたら、Worker を出し直さずに上げられる。変えていなければ、既定の値（$5 と $0.5）。1 つのコードの上限は、出してあるコードの行にも書く（中継も `/gift` の答えも、コードの行の値で見ているので、見る所を増やさない）。全体の上限は、料金を先に足すとき（`reserveBeta`）に設定を読む
- **上限の値**（1 つのコードの 1 日、全体の 1 日、出すコードの数）は、まだ `beta.ts` の定数で、画面からは変えられない
- **手元で画面を見る:** `wrangler dev -c wrangler.admin.jsonc --var ADMIN_DEV_EMAIL:dev@localhost`。ホストが localhost のときだけ Access を通さない（本番のホストはそうならないので、設定を入れても効かない）。キーを足す操作は、API の Worker も手元で動かしているときだけ通る
- **Cloudflare の側の設定（1 度だけ）:** Workers & Pages → `deadlock-jimaku-admin` → Access で、この Worker を守り（All traffic）、入れる人を自分のメールアドレスにする。そこに出る `TEAM_DOMAIN` と `POLICY_AUD` を、この Worker の変数に入れる（`wrangler.admin.jsonc` は `keep_vars` なので、出し直しても消えない）。独自のホスト（`admin.jimaku.deadlock.jp`）を使うなら、Worker の Domains & Routes で足す
- **アドレス:** `https://admin.jimaku.deadlock.jp`（`wrangler.admin.jsonc` の独自のドメイン）と、`https://deadlock-jimaku-admin.chiji.workers.dev`。Access のアプリケーションには、両方のホスト名を入れる（同じアプリケーションに足す。別に作ると `POLICY_AUD` が合わず、Worker が 401 で断る）
- **出し方:** API の Worker を先に（`pnpm deploy`。`Vault` の口が要る）、次に管理画面（`pnpm deploy:admin`）
