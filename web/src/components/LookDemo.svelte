<script lang="ts">
  // 文字の見た目の見本。アプリの字幕とランクの部品に、操作パネルで選べる値を渡して切り替える
  import type { FontStyle } from "../../../frontend/src/generated/FontStyle";
  import type { RankStyle } from "../../../frontend/src/generated/RankStyle";
  import type { SubtitleStyle } from "../../../frontend/src/generated/SubtitleStyle";
  import Captions from "../../../frontend/src/lib/Captions.svelte";
  import RankBadge from "../../../frontend/src/lib/RankBadge.svelte";
  import { display, enriched, LINES, RANK } from "../lib/demo";

  type Look = {
    label: string;
    /** アプリはフォントを lang.css の --font で渡す。LP は lang.css を読まないので、外側の font-family で当てる */
    font?: string;
    color?: string;
    outline?: { width: number; color: string };
    shadow?: { blur: number; color: string };
    band?: boolean;
    weight?: number;
    /** 字間（em） */
    spacing?: number;
  };

  const LOOKS: Look[] = [
    { label: "標準" },
    { label: "帯", band: true },
    {
      label: "ポップ",
      font: "Dela Gothic One",
      color: "#ffe14d",
      outline: { width: 0.24, color: "#2a1600" },
      weight: 400,
    },
    {
      label: "明朝",
      font: "Shippori Mincho B1",
      outline: { width: 0.06, color: "#000000" },
      shadow: { blur: 0.4, color: "#000000" },
      weight: 800,
      spacing: 0.04,
    },
    {
      label: "ドット",
      font: "DotGothic16",
      outline: { width: 0.2, color: "#12306b" },
      weight: 400,
      spacing: 0.02,
    },
  ];

  let chosen = $state(0);
  const look = $derived(LOOKS[chosen]!);

  const text = $derived({
    color: look.color ?? null,
    outline: { width: look.outline?.width ?? null, color: look.outline?.color ?? null },
    shadow: {
      enabled: look.shadow ? true : null,
      blur: look.shadow?.blur ?? null,
      color: look.shadow?.color ?? null,
    },
  });
  const font = $derived<FontStyle>({
    font: look.font ?? null,
    weight: look.weight ?? null,
    letterSpacing: look.spacing ?? null,
  });
  const subtitle = $derived<SubtitleStyle>({
    ...text,
    background: { enabled: look.band ?? false, color: null, opacity: null },
    lineHeight: null,
    languages: { ja: font, en: font },
  });
  const rank = $derived<RankStyle>({ ...text, ...font });
  const segment = enriched("look", LINES[0]!, "en");
</script>

<figure class="reveal">
  <div
    class="plate look relative h-72 overflow-hidden sm:h-80"
    style:font-family={look.font && `"${look.font}", var(--font-sans)`}
  >
    <!-- 一番上の動画の 1 コマから、ゲームの表示（HUD）のないところを拡大して背景にする -->
    <img src="/media/hero.webp" alt="" class="look-scene absolute inset-0 size-full object-cover" loading="lazy" />
    <div class="look-rank" role="img" aria-label="ランクの表示（エミッサリー III）">
      <RankBadge rank={RANK} lang="ja" content="name" look={rank} />
    </div>
    <div class="look-caption">
      <Captions segments={[segment]} display={display(21, 15)} look={subtitle} />
    </div>
  </div>
  <figcaption class="mt-5 flex flex-wrap items-center justify-center gap-2">
    <span class="mr-1 text-sm text-muted">押すと見た目が変わります</span>
    {#each LOOKS as l, i (l.label)}
      <button
        type="button"
        aria-pressed={chosen === i}
        onclick={() => (chosen = i)}
        class="rounded-full border border-line px-4 py-1.5 text-sm font-semibold transition hover:border-ink aria-pressed:border-ink aria-pressed:bg-ink aria-pressed:text-paper"
      >
        {l.label}
      </button>
    {/each}
  </figcaption>
</figure>

<style>
  /* 見本の文字だけを入れたフォント（web/public/media/fonts/OFL.txt）。demo.ts の台詞を変えたら作り直す */
  @font-face {
    font-family: "Dela Gothic One";
    src: url("/media/fonts/dela-gothic-one.woff2") format("woff2");
    font-weight: 400;
    font-display: swap;
  }
  @font-face {
    font-family: "Shippori Mincho B1";
    src: url("/media/fonts/shippori-mincho-b1.woff2") format("woff2");
    font-weight: 800;
    font-display: swap;
  }
  @font-face {
    font-family: "DotGothic16";
    src: url("/media/fonts/dotgothic16.woff2") format("woff2");
    font-weight: 400;
    font-display: swap;
  }
  .look-scene {
    transform: scale(1.9);
    transform-origin: 72% 30%;
  }
  .look-rank {
    --scale: 0.55;
    position: absolute;
    top: 1rem;
    left: 1rem;
  }
  .look-caption {
    position: absolute;
    inset: auto 0 1.25rem 0;
  }
</style>
