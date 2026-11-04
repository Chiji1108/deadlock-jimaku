// LP の見本の字幕。アプリのオーバーレイと同じ流れ（途中の字幕 → ふりがな・色・アイコン → 翻訳）を、用意した台詞で見せる
const NAMES = {
  graves: { kind: "hero", icon: "necro_sm", en: "Graves", ru: "Грейвс", zh: "格瑞墓" },
  theft: { kind: "ability", icon: "necro_siphon", en: "Essence Theft", ru: "Хищение эссенции", zh: "精华转移" },
  split: { kind: "weapon", icon: "titanic_magazine", en: "Split Shot", ru: "Веерный выстрел", zh: "裂光射击" },
  doorman: { kind: "hero", icon: "doorman_sm", en: "The Doorman", ru: "Швейцар", zh: "门侍" },
  cart: { kind: "ability", icon: "doorman_luggagetrolley", en: "Luggage Cart", ru: "Багажная тележка", zh: "行李车" },
  unstoppable: { kind: "vitality", icon: "unstoppable", en: "Unstoppable", ru: "Неудержимость", zh: "势不可挡" },
} as const;

export const LANGS = [
  { value: "en", label: "English" },
  { value: "ru", label: "Русский" },
  { value: "zh", label: "中文" },
] as const;
type Lang = (typeof LANGS)[number]["value"];
type Name = keyof typeof NAMES;

/** 文字列はそのまま、[文字, 読み] はふりがな付き、{ name } は用語（spoken があれば話した言葉で出す） */
type Piece = string | [string, string] | { name: Name; spoken?: Piece[] };
type Line = { source: Piece[]; translations: Record<Lang, Piece[]>; card?: boolean };

export const LINES: Line[] = [
  {
    source: [{ name: "graves", spoken: ["グレイブス"] }, "の", { name: "theft", spoken: ["3", ["番", "ばん"], "スキル"] }, "は", { name: "split", spoken: ["スプリットショット"] }, "で", ["強", "つよ"], "くなる。"],
    translations: {
      en: [{ name: "graves" }, "'s ", { name: "theft" }, " gets stronger with ", { name: "split" }, "."],
      ru: [{ name: "graves" }, ": ", { name: "theft" }, " усиливается предметом ", { name: "split" }, "."],
      zh: [{ name: "graves" }, "的", { name: "theft" }, "会因", { name: "split" }, "而变强。"],
    },
    card: true,
  },
  {
    source: [{ name: "doorman", spoken: ["ドアマン"] }, "の", { name: "cart", spoken: ["カート"] }, "が", ["来", "き"], "た！", ["一回", "いっかい"], ["下", "さ"], "がろう！"],
    translations: {
      en: [{ name: "doorman" }, "'s ", { name: "cart" }, " is coming! Let's fall back!"],
      ru: [{ name: "doorman" }, " — едет ", { name: "cart" }, "! Отступаем!"],
      zh: [{ name: "doorman" }, "的", { name: "cart" }, "来了！先撤一下！"],
    },
  },
  {
    source: [["次", "つぎ"], "は", { name: "unstoppable", spoken: ["アンストッパブル"] }, "を", ["買", "か"], "います！"],
    translations: {
      en: ["Next I'm buying ", { name: "unstoppable" }, "!"],
      ru: ["Дальше покупаю ", { name: "unstoppable" }, "!"],
      zh: ["接下来买", { name: "unstoppable" }, "！"],
    },
  },
];

const plain = (pieces: Piece[]): string =>
  pieces
    .map((p) => {
      if (typeof p === "string") return p;
      if (Array.isArray(p)) return p[0];
      return plain(p.spoken ?? []);
    })
    .join("");

/** 中身はこのファイルの台詞だけなので、エスケープしない */
function html(pieces: Piece[], lang: Lang | "ja"): string {
  return pieces
    .map((p) => {
      if (typeof p === "string") return p;
      if (Array.isArray(p)) return `<ruby>${p[0]}<rt>${p[1]}</rt></ruby>`;
      const n = NAMES[p.name];
      const label = lang === "ja" ? html(p.spoken ?? [], lang) : n[lang];
      return `<span class="entity ${n.kind}"><img src="/media/icons/${n.icon}.webp" alt="" />${label}</span>`;
    })
    .join("");
}

export const sourceHtml = (line: Line) => html(line.source, "ja");
export const translationHtml = (line: Line, lang: Lang) => html(line.translations[lang], lang);
export const spokenText = (line: Line) => plain(line.source);

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** 画面の外にいる間は進めない */
function whenVisible(root: Element) {
  let visible = false;
  let wake = () => {};
  new IntersectionObserver(([entry]) => {
    visible = entry?.isIntersecting ?? false;
    if (visible) wake();
  }).observe(root);
  return () => (visible ? Promise.resolve() : new Promise<void>((resolve) => (wake = resolve)));
}

export function startLiveCaption(root: HTMLElement) {
  const caption = root.querySelector<HTMLElement>("[data-caption]")!;
  const source = root.querySelector<HTMLElement>("[data-source]")!;
  const translation = root.querySelector<HTMLElement>("[data-translation]")!;
  const card = root.querySelector<HTMLElement>("[data-card]");
  const chips = [...document.querySelectorAll<HTMLButtonElement>("[data-lang]")];
  let lang: Lang = "en";
  let current = LINES[0]!;
  let settled = true;

  const showTranslation = () => {
    translation.innerHTML = translationHtml(current, lang);
    translation.classList.remove("show");
    void translation.offsetWidth;
    translation.classList.add("show");
  };

  for (const chip of chips)
    chip.addEventListener("click", () => {
      lang = chip.dataset.lang as Lang;
      for (const c of chips) c.setAttribute("aria-pressed", String(c === chip));
      if (settled) showTranslation();
    });

  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;

  const visible = whenVisible(root);
  void (async () => {
    for (let i = 0; ; i = (i + 1) % LINES.length) {
      await visible();
      current = LINES[i]!;
      settled = false;
      caption.classList.remove("leaving");
      caption.classList.add("pending");
      translation.classList.remove("show");
      translation.textContent = "";
      const chars = [...spokenText(current)];
      for (let n = 1; n <= chars.length; n++) {
        source.textContent = chars.slice(0, n).join("");
        await sleep(62);
      }
      await sleep(420);
      caption.classList.remove("pending");
      source.innerHTML = sourceHtml(current);
      await sleep(360);
      settled = true;
      showTranslation();
      if (current.card) card?.classList.add("show");
      await sleep(4600);
      card?.classList.remove("show");
      caption.classList.add("leaving");
      await sleep(600);
    }
  })();
}
