import type { Segment } from "../generated/Segment";
import type { Token } from "../generated/Token";
import type { Translator } from "./speech";

/** 字幕を組み立てて送る。途中の文もその場で訳す（訳している間に届いた文は、終わってから訳す） */
export class Captions {
  private current?: Segment;
  private seq = 0;
  /** 古い字幕が新しい字幕を追い越さないよう、1 つずつ送る */
  private sending: Promise<unknown> = Promise.resolve();
  private translating = false;

  constructor(
    private lang: string,
    private translators: Map<string, Translator>,
    private send: (segment: Segment) => Promise<unknown>,
  ) {}

  onText(text: string, final: boolean) {
    const trimmed = text.trim();
    if (!trimmed) return;
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
    if (this.translating) return;
    this.translating = true;
    let text;
    do {
      text = segment.text;
      const draft = await this.translate(text);
      if (segment === this.current && Object.keys(draft).length > 0) {
        segment.draft = draft;
        this.publish(segment);
      }
    } while (segment === this.current && segment.text !== text);
    this.translating = false;
  }

  private async settle(segment: Segment) {
    segment.stage = "final";
    this.publish(segment);
    const translations = await this.translate(segment.text);
    const { draft: _, ...settled } = segment;
    this.publish({
      ...settled,
      stage: "enriched",
      enriched: { source: tokens(segment.text), translations },
      settledAt: Date.now(),
    });
  }

  /** 訳せなかった言語は省く（全部だめなら、話した言葉だけを出す） */
  private async translate(text: string) {
    const results = await Promise.all(
      [...this.translators].map(async ([lang, translator]) => {
        const translated = await translator.translate(text).catch(() => "");
        return [lang, tokens(translated)] as const;
      }),
    );
    return Object.fromEntries(results.filter(([, t]) => t.length > 0));
  }
}

const tokens = (text: string): Token[] => (text ? [{ t: "text", text }] : []);
