import type { ChromeTask } from "../generated/ChromeTask";
import type { TermName } from "../generated/TermName";
import type { Token } from "../generated/Token";
import type { Translated } from "../generated/Translated";
import type { Translator } from "./speech";

const joined = (char: string | undefined) => !!char && /[0-9A-Za-z]/.test(char);

/** 別の単語の一部（Rem と Remember）は名前にしない */
function findWord(text: string, word: string) {
  for (let i = text.indexOf(word); i >= 0; i = text.indexOf(word, i + 1)) {
    if (!joined(text[i - 1]) && !joined(text[i + word.length])) return i;
  }
  return -1;
}

/** 訳文に残った英語名を用語にする。原文で見つけた用語だけを探す。同じ位置からは長い名前を取る */
export function markNames(translation: string, terms: TermName[]) {
  const tokens: Token[] = [];
  const text = (part: string) => {
    if (part) tokens.push({ t: "text", text: part });
  };
  let rest = translation;
  for (;;) {
    const next = terms
      .map((term) => ({ term, start: findWord(rest, term.en) }))
      .filter(({ start }) => start >= 0)
      .sort((a, b) => a.start - b.start || b.term.en.length - a.term.en.length)[0];
    if (!next) break;
    text(rest.slice(0, next.start));
    tokens.push({ t: "entity", ref: next.term.ref });
    rest = rest.slice(next.start + next.term.en.length);
  }
  text(rest);
  return tokens;
}

/** core に頼まれた文を訳す。途中の文は最新だけを訳し（訳している間に届いた文は、終わってから訳す）、
 * 確定の文はすべて訳す */
export class Translation {
  private latest?: ChromeTask;
  private translating = false;

  constructor(
    private translators: Map<string, Translator>,
    private send: (translated: Translated) => void,
  ) {}

  onTask(task: ChromeTask) {
    if (task.final) {
      if (this.latest?.id === task.id) this.latest = undefined;
      void this.translate(task);
      return;
    }
    this.latest = task;
    void this.draft();
  }

  private async draft() {
    if (this.translating) return;
    this.translating = true;
    while (this.latest) {
      const task = this.latest;
      this.latest = undefined;
      await this.translate(task);
    }
    this.translating = false;
  }

  /** 訳せなかった言語は省く */
  private async translate({ id, text, terms, final }: ChromeTask) {
    const results = await Promise.all(
      [...this.translators].map(async ([lang, translator]) => {
        const translated = await translator.translate(text).catch(() => "");
        return [lang, markNames(translated, terms)] as const;
      }),
    );
    const translations = Object.fromEntries(results.filter(([, tokens]) => tokens.length > 0));
    this.send({ id, final, translations });
  }
}
