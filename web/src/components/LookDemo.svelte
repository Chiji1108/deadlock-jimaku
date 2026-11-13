<script lang="ts">
  // 文字の見た目の見本。アプリの字幕とランクの部品に、操作パネルで選べる値を渡して切り替える
  import { onMount } from "svelte";
  import type { RankStyle } from "../../../frontend/src/generated/RankStyle";
  import type { SubtitleStyle } from "../../../frontend/src/generated/SubtitleStyle";
  import Captions from "../../../frontend/src/lib/Captions.svelte";
  import RankBadge from "../../../frontend/src/lib/RankBadge.svelte";
  import scene from "../assets/hero.webp?url";
  import { display, enriched, LINES, RANK } from "../lib/demo";
  import { fontFile, LOOKS } from "../lib/looks";

  const FILES = import.meta.glob<string>("../assets/fonts/*.woff2", {
    eager: true,
    import: "default",
    query: "?url",
  });

  let chosen = $state(0);
  const look = $derived(LOOKS[chosen]!);
  const subtitle = $derived<SubtitleStyle>({
    ...look.text,
    background: look.background,
    lineHeight: null,
    languages: { ja: look.fontStyle, en: look.english },
  });
  const rank = $derived<RankStyle>({ ...look.text, ...look.fontStyle });
  const segment = enriched("look", LINES[0]!, "en");

  /** 見た目ごとの、読み込むフォント（日本語と英語） */
  let faces: FontFace[][] = [];

  // 画面に入って動き始めたら（client:visible）見本のフォントを全部読んでおく
  onMount(() => {
    faces = LOOKS.map((l) =>
      [l.fontStyle, l.english].flatMap(({ font, weight }) => {
        if (!font || !weight) return [];
        const url = FILES[`../assets/fonts/${fontFile(font, weight)}`];
        const face = new FontFace(font, `url(${url})`, { weight: String(weight) });
        document.fonts.add(face);
        void face.load().catch(() => {});
        return [face];
      }),
    );
  });

  /** 読み終えてから切り替え、別のフォントで一瞬出さない。読めなければ標準の候補で出す */
  async function choose(index: number) {
    await Promise.all((faces[index] ?? []).map((face) => face.loaded.catch(() => {})));
    chosen = index;
  }
</script>

<figure class="reveal">
  <div class="plate plate-rank relative h-72 overflow-hidden sm:h-80">
    <!-- 一番上の動画の 1 コマから、ゲームの表示（HUD）のないところを拡大して背景にする -->
    <img src={scene} alt="" class="scene absolute inset-0 size-full object-cover" loading="lazy" />
    <div class="absolute top-4 left-4" role="img" aria-label="ランクの表示（エミッサリー III）">
      <RankBadge rank={RANK} lang="ja" content="name" look={rank} />
    </div>
    <div class="plate-caption">
      <Captions segments={[segment]} display={display(21, 15)} look={subtitle} />
    </div>
  </div>
  <figcaption class="mt-5 flex flex-wrap items-center justify-center gap-2">
    <span class="mr-1 text-sm text-muted">押すと見た目が変わります</span>
    {#each LOOKS as l, i (l.label)}
      <button
        type="button"
        aria-pressed={chosen === i}
        onclick={() => choose(i)}
        class="rounded-full border border-line px-4 py-1.5 text-sm font-semibold transition hover:border-ink aria-pressed:border-ink aria-pressed:bg-ink aria-pressed:text-paper"
      >
        {l.label}
      </button>
    {/each}
  </figcaption>
</figure>

<style>
  .scene {
    transform: scale(1.9);
    transform-origin: 72% 30%;
  }
</style>
