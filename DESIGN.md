# deadlock-stream-overlays 設計書

Deadlock 配信者向けの OBS オーバーレイツール。

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
- 音声認識: Mistral Voxtral 3（Realtime と Small Transcribe 3）
- 翻訳: Mistral Small 4（Large 4 は遅すぎるので使わない。9.3）
- ふりがな: 形態素解析（Lindera + IPADIC）、固有名詞: 辞書との照合（9 章）
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
| Mistral | `reqwest` + `tokio-tungstenite` | 公式の Rust SDK がないので直接呼ぶ |
| API キー | `keyring` | Windows 資格情報マネージャー |
| 設定 | JSON（アプリの設定ディレクトリ） | |
| フロント | **Svelte 5 (runes) + Vite + TypeScript** | 下記 |
| スタイル | 素の CSS + CSS 変数 | OBS の「カスタム CSS」で見た目を上書きできる |

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
| `mise run headless` | Tauri なしで中核だけ起動し、ダミー字幕を流し続ける（`--demo --translate-to en,ru`。`http://127.0.0.1:4510/overlay/subtitles`） |

ヘッドレスの `overlays-headless` は `--mic` / `--mic-device NAME` でマイクを、`--input-file PATH.wav` で録音を実時間で流して字幕にできる（どちらも環境変数 `MISTRAL_API_KEY` が必要）。Tauri アプリは `MISTRAL_API_KEY` があれば起動時に既定のマイクで字幕を始める（キーの保存と開始・停止の操作は Phase 5）。
| `mise run check` | oxfmt / oxlint / svelte-check / rustfmt / clippy / cargo test |

- Git フック（**lefthook**）: commit 時に変更ファイルへ oxfmt / oxlint / cargo fmt、push 時に `mise run check`。clone 後に `mise exec -- lefthook install` を一度実行する。
- 整形とリント: フロントは **oxfmt**（`svelte: true` で `.svelte` も整形）と **oxlint**、Rust は rustfmt と clippy。
- TypeScript は 6 系に固定する（svelte-check がまだ 7 系に対応していない）。
- `frontend/src/generated/` は ts-rs の出力（`cargo test -p overlays-core` で更新）。手で編集しない。

## 3. Mistral API（2026-11 時点のドキュメントで確認）

| 用途 | モデル ID | 料金 | 備考 |
|---|---|---|---|
| partial | `voxtral-mini-transcribe-realtime-3` | $0.008/分 | WebSocket。用語ヒントに対応 |
| final | `voxtral-small-transcribe-3`（`-latest` はまだ v2 を指すので使わない） | $0.004/分 | `/v1/audio/transcriptions`。精度が最も高い |
| enrich（翻訳） | `mistral-small-2603` | — | 推論なしで 1 発話 1〜3 秒。構造化出力に対応 |
| （不採用） | `mistral-large-4` | 100 万トークンあたり 入力 $0.70 / キャッシュ済み入力 $0.07 / 出力 $2.10 | 推論を止められず 1 発話 21〜24 秒（9.3） |

モデル ID はすべて設定で持つ（enrich を Mistral Small 4 に差し替えられるように）。

### 3.1 Realtime（WebSocket）

- URL: `wss://api.mistral.ai/v1/audio/transcriptions/realtime?model=voxtral-mini-transcribe-realtime-3`
- 認証: `Authorization: Bearer <key>` ヘッダー。ブラウザの WebSocket ではヘッダーを付けられないので、Rust の中核から接続する。
- 流れ: 接続 → `session.created` → `session.update` → `input_audio.append`（base64）を繰り返す → `input_audio.end` → `transcription.done`
- **`session.update` は最初の音声より前に 1 回だけ送れる**。音声を送り始めたら設定は変えられない。

```json
{
  "type": "session.update",
  "session": {
    "audio_format": { "encoding": "pcm_s16le", "sample_rate": 16000 },
    "target_streaming_delay_ms": 480,
    "language": "ja",
    "context_bias": ["インファーナス", "基礎拡張マガジン", "..."]
  }
}
```

| フィールド | 値 |
|---|---|
| `target_streaming_delay_ms` | 80〜1200 の整数、または 2400。推奨の出発点は 480。短いほど速く、長いほど正確 |
| `language` | ISO 639-1。省略すると自動判定 |
| `context_bias` | 文字列の配列（キーフレーズ、または自由な説明文）。件数の上限はないが、音声とコンテキストを共有する |
| `turn_detection` | 本ツールでは使わない（下記）。参考: `min_silence_ms`(500) / `end_timeout_ms`(3000) / `start_threshold`(0.5) / `eager_end_threshold`(0.5) / `end_threshold`(0.33) |

サーバーからのイベント:

| type | 内容 |
|---|---|
| `transcription.text.delta` | `text`。**過去のテキストは訂正されない（追記のみ）** |
| `turn.start` / `turn.eager_end` / `turn.resume` / `turn.end` | `time_s`。`turn.end` には `end_reason`（`threshold` / `timeout`）が付く |
| `transcription.done` | セッション終了時の全文 |
| `error` | `error.message`, `error.code` |

**発話の区切り方: ターン検出は使わない**

ターン検出は「会話で誰が話す番か」を判定する機能で、字幕の区切り（文や息継ぎの単位）とは目的が違う。検証（16 章）では、0.4〜1.5 秒の間を挟みながら 22 秒話し続けた音声が、**既定値でも積極的な設定でも 1 ターン**になった。配信者は何分も話し続けるので、ターンの終わりを待つと final と翻訳がいつまでも出ない。

そこで区切りはローカルの VAD（earshot）で決める。

- 無音が `minSilenceMs`（既定 400ms）続いたら Segment を閉じる
- 長さが `maxSegmentSeconds`（既定 8 秒）を超えたら、次の短い無音で強制的に閉じる（字幕が長くなりすぎないように）
- 音声は手元にあるので、区切りの位置が正確にわかり、追加のコストもかからない

`session.update` では `turn_detection` を送らない。

### 3.2 Small Transcribe 3（バッチ）

- パラメータ: `model`, `file`, `language`, `context_bias`, `timestamp_granularities`（v3 では `segment` のみ。`word` を指定すると 400 エラー）
- ターンの音声区間を切り出して送る。`context_bias` は realtime と同じものを渡す。

### 3.3 1 時間あたりのコスト見積もり

| 項目 | 見積もり |
|---|---|
| Realtime（常時接続） | 60 分 × $0.008 = **約 $0.48** |
| Small Transcribe 3（発話区間のみ。仮に 40%） | 24 分 × $0.004 = **約 $0.10** |
| Large 4（1 時間に約 600 発話。辞書 8k トークンはキャッシュ、その他の入力 500 / 出力は翻訳先 1 言語につき 150 トークン） | 翻訳先 1 言語で **約 $0.6**、1 言語増えるごとに **+約 $0.2** |
| 合計（翻訳先が英語のみ） | **約 $1.2/時間** |

辞書をプロンプトの固定の先頭部分に置いてキャッシュを効かせるのが、コストに一番効く。

※ この表は Large 4 で見積もったもの。翻訳は Small 4 に変え、出力も訳文だけにしたので、実際はこれより安い（料金は Mistral の価格表で確認する）。

## 4. リポジトリ構成

```
deadlock-stream-overlays/
├─ DESIGN.md
├─ Cargo.toml                 # workspace
├─ crates/
│  └─ core/                   # Tauri に依存しない。単体でヘッドレス起動・テストできる
│     └─ src/
│        ├─ audio.rs          # マイク → 16kHz mono フレーム + リングバッファ
│        ├─ stt/              # Voxtral の realtime / batch クライアント
│        ├─ enrich/           # Mistral Large の呼び出し・出力の検証
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

## 5. 設定

```ts
interface Settings {
  mic: {
    device: string;               // cpal のデバイス名
    language: LangCode;           // 配信者が話す言語。既定は "ja"
  };
  translateTo: LangCode[];        // 既定は ["en"]。最大 4 言語。mic.language は除く
  realtime: {
    targetStreamingDelayMs: number;  // 既定 480
  };
  segmentation: {
    minSilenceMs: number;            // 既定 400
    maxSegmentSeconds: number;       // 既定 8
  };
  models: { realtime: string; batch: string; enrich: string };
  rank: { accountId?: number; pollSeconds: number };  // 既定 180
  display: { ttlSeconds: number; perCharSeconds: number };  // 既定 6, 0.15
}
```

- `translateTo` を変えると、次の発話から反映される。
- `mic.language` を変えたら、Realtime セッションを張り直す（設定は最初に 1 回しか送れないため）。

## 6. 言語

### 6.1 方針

- 内部の言語コードは BCP 47。deadlock-api の言語名と Voxtral の ISO 639-1 コードは対応表で変換する。
- **配信者が話す言語（`mic.language`）も翻訳先も、対応表のどれでも選べる**。日本語の配信者を前提にしているので既定は `ja` だが、言語ごとの分岐はコードに書かない。
- 制約になるのは deadlock-api（公式名がある言語）と LLM の翻訳の質だけ。音声認識の対応言語が効くのは `mic.language` だけ。

### 6.2 対応表（`lang.rs` → ts-rs で TS に出す）

| LangCode | deadlock-api | Voxtral | 表示名 |
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

- ルビは**日本語のテキストにだけ**付ける。配信者が日本語で話すとき（既定）は原文に付き、翻訳先に `ja` を選んだときはその訳文に付く。

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
     └─▶ Voxtral Realtime（WS。セッションを張りっぱなしにする）
           │ transcription.text.delta → 割り当て先の Segment に upsert(stage=partial, text += delta)
           ▼
  Segment が閉じたら、リングバッファから区間 [start-300ms, end+300ms] を切り出す
         ▼
  Small Transcribe 3（バッチ）+ context_bias  → upsert(stage=final, text)
         │   失敗したとき: realtime で溜めたテキストを final にする
         ▼
  Mistral Large 4（構造化出力）
         ├─ kind=speech → upsert(stage=enriched, enriched, settledAt)
         ├─ kind=noise  → upsert(stage=discarded)
         └─ 失敗・タイムアウト → final のまま settledAt を付ける（翻訳なしで表示して消える）
```

- **区切りはローカルの VAD で決める**（3.1 の「発話の区切り方」を参照）。
- **delta の割り当て**: delta にはタイムスタンプがないので、「まだ partial を受け付けている Segment のうち最も古いもの」に追記する。Segment は閉じてから `targetStreamingDelayMs + 1.5 秒` の間だけ delta を受け付け、それを過ぎたら次の Segment に回す。境界で 1〜2 語ずれても、数秒後に final で置き換わるので許容する。
- Segment ごとに独立した非同期タスクで処理する。enrich の完了順が前後しても、表示順は `startedAt` で固定する。
- enrich には直前 2〜3 発話の final テキストを文脈として渡す（代名詞や話の流れのため）。
- Realtime セッションが切れたら、新しいセッションを張り直す（開いている Segment はその時点の内容で final に進める）。

### 7.3 遅延の目安

| 段階 | 目安 |
|---|---|
| partial | 1 秒未満（streaming delay 480ms） |
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
4. 読みが登録されていない漢字の名前には、形態素解析で読みを付ける（client 6839 で 35 件すべてに付いた）。辞書が知らない造語（人狼・屍鬼・魔呪・五芒星など）で読みが誤るものは `readings.ja.json` で直す（今は 7 件）。

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

`context_bias` には、`mic.language` での名前と略称を入れる（およそ 400 件）。音声とコンテキストを共有するので、多すぎて 400 エラーになるときは、ヒーローとアイテムを優先してスキルを削る。

## 9. enrich（`enrich.rs`, `furigana.rs`）

### 9.1 方針: LLM には翻訳だけを任せる

最初は「トークン分け・ふりがな・固有名詞・翻訳・雑音の判定」をすべて LLM に返させたが、実測で次の問題が出たので役割を分けた（16 章）。

| 問題（Mistral Small 4） | 対応 |
|---|---|
| ふりがながずれる（`僕(ぼ)`、`最(さい)近(き)`）、送り仮名ごと付ける（`強い(つよい)`） | **形態素解析**（Lindera + IPADIC、埋め込み辞書）で読みを決める。1 文 1ms 程度 |
| 名前の途中で区切る（`ヘ⟨イズ⟩`）、同じ名前を付けたり付けなかったりする | **辞書の名前と略称の最長一致**で固有名詞を決める |
| 普通の文を「雑音」と判定して消す | 判定しない。咳やフィラーだけの音声はバッチの文字起こしが空になり、そこで捨てる |
| 訳文で `{id}` を使わず名前をそのまま書く（`Haze's`） | 原文に出てきたエンティティの、その言語と英語の名前を訳文から探してエンティティに戻す |

### 9.2 処理の流れ

1. final のテキストを、辞書の名前と略称（話す言語、2 文字以上）で最長一致に分ける。カタカナの名前は、前後もカタカナなら別の語の一部として一致させない（「セブンイレブン」の「セブン」）
2. 固有名詞以外の部分を形態素解析し、漢字を含む語にルビを付ける（送り仮名と接頭のかなは外に出す。「強(つよ)い」「お金(かね)」）。日本語のときだけ
3. LLM に `{id|発話された表現}` 入りの原文と直前 3 発話を渡し、各言語の訳文を `{id}` 入りで返させる
4. 訳文を `{id}` で区切り、辞書にない ID は捨て、名前のまま書かれたものはエンティティに戻す

翻訳に失敗しても、1・2 の結果（固有名詞とふりがな）は付けて enriched にする。

### 9.3 LLM の入出力

- モデル: `mistral-small-2603`（設定の `models.enrich`）
  - Mistral Large 4 は推論を止められず（`reasoning_effort` は low / medium / high のみ）、1 発話に 21〜24 秒かかり、出力が途中で切れることもあったので使わない
- system: 役割とルール、ゲーム用語の一覧（`id: 英語名`。固定の先頭部分にしてキャッシュを効かせる）
- user: `{ "context": [直前の確定字幕], "transcript": "{hero:13|ヘイズ}のウルト強い" }`
- 構造化出力（`response_format: json_schema`, `strict`）: `{ "translations": { "en": "...", "ru": "..." } }`。`translations` のキーは設定の翻訳先から毎回作る
- タイムアウト 8 秒

### 9.4 出力の型

```ts
type Token =
  | { t: "text"; text: string; ruby?: string }      // ruby は日本語の漢字を含む語にだけ
  | { t: "entity"; ref: string; spoken?: string };  // spoken = 原文で発話された表現

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
type ClientMsg = { type: "subscribe"; topics: ("subtitle" | "glossary" | "rank")[] };

// server → client
type ServerMsg =
  | { type: "snapshot"; segments: Segment[]; rank?: RankState }  // 接続直後
  | { type: "segment.upsert"; segment: Segment }
  | { type: "rank.update"; rank: RankState }
  | { type: "glossary.updated"; clientVersion: number };          // オーバーレイが辞書を取り直す
```

- 辞書は `GET /api/glossary` で取得し、オーバーレイ側で持つ（Segment には ID しか載らない）。
- サーバーは直近の Segment を（例: 50 件）保持して、snapshot で送る。
- WS クライアントは指数バックオフで再接続し、再接続したら snapshot で状態を置き換える。
- 接続直後と broadcast の取りこぼし（Lagged）の後は、購読しているすべてのトピックの現在の状態（`snapshot` と、辞書があれば `glossary.updated`）を送る。

## 11. 字幕オーバーレイ

URL: `/overlay/subtitles?langs=en&furigana=1&maxItems=3`

| パラメータ | 意味 | 既定値 |
|---|---|---|
| `langs` | 表示する訳文の言語（`translateTo` の中から選ぶ） | `translateTo` すべて |
| `source` | 原文を表示するか | `1` |
| `furigana` | ふりがなを表示するか | `1` |
| `maxItems` | 同時に表示する Segment の上限 | `3` |

言語ごとにブラウザソースを分ければ、別々の位置に表示できる（例: 原文だけのソースと `?source=0&langs=en` のソース）。

### 表示ルール

- 映画字幕のような**中央揃え**。下から積み上げ、最新の発話を一番下に出す。partial の行は下端でその場で更新される。
- 表示時間は `settledAt` から数え、`ttlSeconds + 文字数 × perCharSeconds` が過ぎたらフェードアウトする。partial / final の間は消さない。
- `maxItems` を超えたら、時間に関係なく古いものから上に押し出す（`animate:slide`。`lib/slide.ts`）。
  - `animate:flip` は使わない。flip は大きさの変化も scale で補間するので、partial → final で行の幅が変わると文字が潰れて見える。
- `discarded` になったらすぐに消す。
- 画面上の位置は OBS 側で決める。Deadlock は画面下中央に HUD があるので、その少し上に置く想定。

### 1 発話の中身

```
   [アイコン]インファーナスのultに気(き)をつけて      ← 原文（大きく。ふりがなは <ruby> で上に出す）
   Watch out for [アイコン]Infernus's ult             ← 訳文（小さく。langs の分だけ並べる）
```

- エンティティは `[アイコン] 公式名` をハイライトして表示する（色は種類ごと: hero / item / ability）。
- partial は灰色、final は通常の色（翻訳待ち）、enriched で完成形になる。

## 12. ランクオーバーレイ

URL: `/overlay/rank?lang=ja`

| 用途 | エンドポイント |
|---|---|
| 現在のランク | `GET /v1/players/{account_id}/rank`（最後のランクマッチが終わった時点のランク。`last_match` が付く） |
| ランク名・バッジ | `GET /v1/assets/ranks?language=…`（辞書と一緒にキャッシュする） |
| 当日の勝敗（任意） | `GET /v1/players/{account_id}/match-history` |

- `account_id` は操作パネルで設定する（Steam の 32bit アカウント ID）。
- `rank.pollSeconds`（既定 180 秒）ごとにポーリングする。データの取り込みに時間がかかるので、試合が終わってすぐには反映されない（deadlock-api の bot とフレンドになるか、Patreon の優先取り込みで速くなる）。
- `RankState { badge, tier, subrank, names: Record<LangCode, string>, color, image, updatedAt, today?: { win, loss } }`
- ランクが上がったり下がったりしたらアニメーションする。

## 13. 操作パネル（control.html）

- マイクデバイスの選択とレベルメーター、話す言語（`mic.language`）
- 翻訳先の言語（最大 4 つ。選ぶとコストの目安を表示する）
- Mistral API キー（keyring に保存し、画面には出さない）とモデル ID
- 辞書の状態（clientVersion、取得済みの言語、未確認の読みの一覧）
- ランク: account_id
- オーバーレイ URL のコピー（パラメータを組み立てる UI）
- 直近の Segment のログ（デバッグ用。段階ごとにかかった時間を表示する）

## 14. 実装フェーズ

| Phase | 内容 | 完了条件 |
|---|---|---|
| 0 | 土台: workspace、Tauri + axum + Svelte、WS upsert、ダミーの Segment ✅ | OBS にダミー字幕が流れて消える |
| 1 | 言語の対応表、assets（glossary 生成、アイコンのキャッシュ）、Rich の描画 ✅ | ダミーの Enriched がアイコン・ルビ付きで、複数の言語で表示される |
| 2 | mic → VAD で区切り → Voxtral Realtime（partial）→ Small Transcribe 3（final） ✅ | 自分の声が字幕になる |
| 3 | enrich（ふりがな、翻訳、エンティティ） ✅ | 3 段階の表示が通しで動く |
| 4 | ランクオーバーレイ | ランクバッジが表示され、更新される |
| 5 | 仕上げ: 操作パネル、インストーラー、自動更新 | 配布できる |

Phase 2 の前に、録音した実況音声で Voxtral 3 の精度（`context_bias` あり / なし、realtime / batch）を確かめる。

## 15. 将来の拡張案（今はスコープ外）

- **ゲーム内 VC・Discord の字幕と逆翻訳**: Windows のプロセス単位の録音（WASAPI Process Loopback）で取れる。Realtime の話者分離（`diarize`, `speaker_references`）が使える。外した理由は、チームメイトの同意、キャラのボイスや効果音の混入による精度の限界、コストと複雑さが倍になること。
- **チャット形式のバブル表示**: 上の機能を入れるときに、左右の配置と一緒に入れる。
- **話す言語の自動判定**: Voxtral は `language` を省略すると自動で判定する。ただし短い発話や英単語の多い日本語では誤判定しやすく、用語ヒントの言語も決められなくなるので、今は明示的に設定する。
- **日本語以外の読み補助**（中国語のピンインなど）: Rich の `ruby` の仕組みをそのまま使える。

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
- [ ] 本番に近い条件（配信用マイク、プレイしながら 5〜10 分、用語を多めに）でもう一度測る。`context_bias` の効果はこの条件でないと見えない
- [ ] Realtime セッションの最大接続時間と、切れたときの挙動
- [x] Large 4 / Small 4 の比較（同じ文で Large 4 は 21〜24 秒・推論 2,400〜2,900 トークン、Small 4 は 3 秒・推論なし）→ Small 4 を採用
- [x] `response_format: { type: "json_schema", json_schema: { name, strict, schema } }` が使える
- [x] enrich を通しで確認（sample01）: 4 発話すべてが enriched になり、ふりがな・固有名詞は正しく、final から 0.5〜2 秒
- [ ] プロンプトキャッシュが効いているか（`usage.prompt_tokens_details.cached_tokens`）を配信の長さで確認する
- [ ] 翻訳の質: 英語・ロシア語・中国語以外の言語で、ゲーム実況の口語がどこまで自然に訳せるか
- [ ] ヒーローの `items` から signature スキルを引くときの対応関係
