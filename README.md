# Deadlock Stream Overlays

Deadlock の配信者向けの OBS オーバーレイ。

- **字幕**: マイクの声をリアルタイムに字幕にし、ふりがなと翻訳（最大 4 言語）を付ける。ヒーロー・アイテム・スキルの名前はアイコン付きで強調する
- **ランク**: ランクのアイコン・名前・ポイントのゲージ
- **アイテム説明**: 字幕でアイテム名が出たら、その説明をゲーム内と同じカードで出す

音声認識と翻訳には OpenAI の API キーが必要です。設定はアプリの操作パネルから行います。

設計は [DESIGN.md](DESIGN.md) を見てください。

## 開発

```bash
mise install
pnpm --dir frontend install
mise run dev
```

## ライセンス

[PolyForm Strict License 1.0.0](LICENSE.md)。ソースは公開していますが、改変や再配布はできません。個人の利用はできます（詳しくは本文を見てください）。
