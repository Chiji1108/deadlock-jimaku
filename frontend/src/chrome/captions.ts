import type { Segment } from "../generated/Segment";
import { tokens, translateAll } from "./live";
import type { Translator } from "./speech";

/** 字幕を組み立てて送る。途中の文もその場で訳す（訳している間に届いた文は、終わってから訳す） */
export class Captions {
  private current?: Segment;
  private seq = 0;
  /** 古い字幕が新しい字幕を追い越さないよう、1 つずつ送る */
  private sending: Promise<unknown> = Promise.resolve();
  private translating?: Segment;

  constructor(
    private lang: string,
    private translators: Map<string, Translator>,
    private send: (segment: Segment) => Promise<unknown>,
  ) {}

  onText(text: string, final: boolean) {
    const trimmed = text.trim();
    if (!trimmed) {
      if (final) this.discard();
      return;
    }
    const segment = (this.current ??= {
      id: `chrome-${Date.now()}-${++this.seq}`,
      startedAt: Date.now(),
      stage: "partial",
      lang: this.lang,
      text: "",
    });
    segment.text = trimmed;
    this.publish(segment);
    if (final) {
      this.current = undefined;
      void this.settle(segment);
    } else {
      void this.draft(segment);
    }
  }

  /** 認識が終わったら、確定しなかった文をそのまま確定する */
  finish() {
    if (this.current) this.onText(this.current.text, true);
  }

  /** 止めたら、確定しなかった字幕を消す */
  discard() {
    if (this.current) this.publish({ ...this.current, stage: "discarded" });
    this.current = undefined;
  }

  private publish(segment: Segment) {
    const copy = { ...segment };
    this.sending = this.sending.then(() => this.send(copy)).catch(() => {});
  }

  private async draft(segment: Segment) {
    if (this.translating === segment) return;
    this.translating = segment;
    let text;
    do {
      text = segment.text;
      const draft = await translateAll(this.translators, text);
      if (segment === this.current && Object.keys(draft).length > 0) {
        segment.draft = draft;
        this.publish(segment);
      }
    } while (segment === this.current && segment.text !== text);
    if (this.translating === segment) this.translating = undefined;
  }

  private async settle(segment: Segment) {
    segment.stage = "final";
    this.publish(segment);
    const translations = await translateAll(this.translators, segment.text);
    const { draft: _, ...settled } = segment;
    this.publish({
      ...settled,
      stage: "enriched",
      enriched: { source: tokens(segment.text), translations },
      settledAt: Date.now(),
    });
  }
}
