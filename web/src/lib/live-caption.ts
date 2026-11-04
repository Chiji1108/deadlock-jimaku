// LP の見本の字幕。アプリのオーバーレイと同じ流れ（途中の字幕 → ふりがな・色・アイコン → 翻訳）を、用意した台詞で見せる
const NAMES = {
  doorman: { kind: "hero", icon: "doorman_sm", en: "The Doorman", ru: "Швейцар", zh: "门侍" },
  cart: { kind: "ability", icon: "doorman_luggagetrolley", en: "Luggage Cart", ru: "Багажная тележка", zh: "行李车" },
  dispel: { kind: "vitality", icon: "debuff_remover", en: "Dispel Magic", ru: "Развеивание магии", zh: "驱散魔法" },
  vortex: { kind: "spirit", icon: "vortex_web", en: "Vortex Web", ru: "Паутинный вихрь", zh: "雷织漩涡" },
  indomitable: { kind: "vitality", icon: "indomitable", en: "Indomitable", ru: "Непреклонность", zh: "不屈之志" },
  bebop: { kind: "hero", icon: "bebop_sm", en: "Bebop", ru: "Бибоп", zh: "比波普" },
  krill: { kind: "hero", icon: "digger_sm", en: "Mo & Krill", ru: "Мо и Мелкий", zh: "莫克双雄" },
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
/** card は deadlock-ui のカードに出すアイテムの class_name */
type Line = { source: Piece[]; translations: Record<Lang, Piece[]>; card: string };

export const LINES: Line[] = [
  {
    source: [{ name: "doorman", spoken: ["ドアマン"] }, "の", { name: "cart", spoken: ["カート"] }, "は", { name: "dispel", spoken: ["ディスペル"] }, "で", ["対策", "たいさく"], "できます。"],
    translations: {
      en: [{ name: "doorman" }, "'s ", { name: "cart" }, " can be countered with ", { name: "dispel" }, "."],
      ru: [{ name: "doorman" }, ": ", { name: "cart" }, " контрится предметом ", { name: "dispel" }, "."],
      zh: [{ name: "doorman" }, "的", { name: "cart" }, "可以用", { name: "dispel" }, "应对。"],
    },
    card: "upgrade_reduce_debuff_duration",
  },
  {
    source: ["ただし、", { name: "vortex", spoken: ["ボルテックスウェブ"] }, "には", { name: "indomitable", spoken: ["インドミ"] }, "が", ["必要", "ひつよう"], "です。"],
    translations: {
      en: ["However, you'll need ", { name: "indomitable" }, " for ", { name: "vortex" }, "."],
      ru: ["Но против предмета ", { name: "vortex" }, " нужна ", { name: "indomitable" }, "."],
      zh: ["不过，", { name: "vortex" }, "需要用", { name: "indomitable" }, "来应对。"],
    },
    card: "upgrade_auto_cleanse",
  },
  {
    source: [["敵", "てき"], "に", { name: "bebop", spoken: ["ビバップ"] }, "や", { name: "krill", spoken: ["モークリ"] }, "もいるなら", ["絶対", "ぜったい"], "に", ["買", "か"], "いましょう。"],
    translations: {
      en: ["If the enemy team also has ", { name: "bebop" }, " or ", { name: "krill" }, ", definitely buy it."],
      ru: ["Если у врагов есть ещё ", { name: "bebop" }, " или ", { name: "krill" }, ", обязательно покупайте."],
      zh: ["如果敌方还有", { name: "bebop" }, "或", { name: "krill" }, "，一定要买。"],
    },
    card: "upgrade_auto_cleanse",
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

/** deadlock-ui のカードの幅 */
const CARD_WIDTH = 450;

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
  const card = root.querySelector<HTMLElement>("[data-card]")!;
  const tooltip = card.querySelector("dl-item-tooltip")!;
  const chips = [...document.querySelectorAll<HTMLButtonElement>("[data-lang]")];
  let lang: Lang = "en";
  let current = LINES[0]!;
  let settled = true;

  // カードは幅の決まった部品なので、枠の幅に合わせて縮める
  new ResizeObserver(() => {
    tooltip.style.setProperty("zoom", String(card.clientWidth / CARD_WIDTH));
  }).observe(card);

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
      tooltip.setAttribute("class-name", current.card);
      card.classList.add("show");
      await sleep(4600);
      if (LINES[(i + 1) % LINES.length]!.card !== current.card) card.classList.remove("show");
      caption.classList.add("leaving");
      await sleep(600);
    }
  })();
}
