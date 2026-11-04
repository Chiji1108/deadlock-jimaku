<script lang="ts">
  // 一番上の配信画面の見本。アプリの字幕とランクの部品に、用意した台詞を流す
  import { onMount } from "svelte";
  import Captions from "../../../frontend/src/lib/Captions.svelte";
  import RankBadge from "../../../frontend/src/lib/RankBadge.svelte";
  import { display, enriched, LANGS, LINES, partial, RANK, spokenText } from "../lib/demo";

  /** 配信画面の幅。部品の大きさは OBS の 1920px の画面に置いたときの比で決める */
  const SCREEN = 1920;
  /** deadlock-ui のカードの幅 */
  const CARD_WIDTH = 450;

  let width = $state(1152);
  let lang = $state<(typeof LANGS)[number]["value"]>("en");
  let index = $state(0);
  let text = $state<string | undefined>();
  let cycle = $state(0);
  let card = $state<string | undefined>();
  let settled = $state(true);

  const narrow = $derived(width < 640);
  const line = $derived(LINES[index]!);
  const segments = $derived(
    text === undefined
      ? []
      : settled
        ? [enriched(`line-${cycle}`, line, lang)]
        : [partial(`line-${cycle}`, text)],
  );
  const scale = $derived(narrow ? width / 900 : width / SCREEN);
  const subtitle = $derived(
    narrow ? display(width * 0.054, width * 0.036) : display(width * 0.0255, width * 0.0165),
  );

  const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

  onMount(() => {
    // カードの部品は重いので、ページが出てから読み込む
    void Promise.all([
      import("@deadlock-api/ui-core/dist/components/dl-provider.js"),
      import("@deadlock-api/ui-core/dist/components/dl-item-tooltip.js"),
    ]).then(([provider, tooltip]) => {
      provider.defineCustomElement();
      tooltip.defineCustomElement();
    });

    if (matchMedia("(prefers-reduced-motion: reduce)").matches) {
      text = spokenText(line);
      return;
    }
    let stopped = false;
    void (async () => {
      for (let i = 0; !stopped; i = (i + 1) % LINES.length) {
        index = i;
        cycle += 1;
        settled = false;
        const chars = [...spokenText(LINES[i]!)];
        for (let n = 1; n <= chars.length && !stopped; n++) {
          text = chars.slice(0, n).join("");
          await sleep(62);
        }
        await sleep(420);
        settled = true;
        card = LINES[i]!.card;
        await sleep(5000);
        if (LINES[(i + 1) % LINES.length]!.card !== card) card = undefined;
        text = undefined;
        await sleep(600);
      }
    })();
    return () => {
      stopped = true;
    };
  });
</script>

<figure class="hero-stage mx-auto mt-8">
  <div class="stream" bind:clientWidth={width}>
    <video
      src="/media/hero.mp4"
      poster="/media/hero.webp"
      autoplay
      muted
      loop
      playsinline
      aria-label="Deadlock の配信画面。左上にランク、右にアイテム説明、下の中央に字幕が重なる見本"
    ></video>
    <div class="stream-rank" style:--scale={scale}>
      <RankBadge rank={RANK} lang="ja" content="full" />
    </div>
    <dl-provider language="japanese">
      <div class="stream-card" class:show={card} style:zoom={scale * (narrow ? 0.9 : 1)}>
        {#if card}<dl-item-tooltip class-name={card}></dl-item-tooltip>{/if}
      </div>
    </dl-provider>
    <div class="stream-captions"><Captions {segments} display={subtitle} /></div>
  </div>
  <figcaption class="mt-5 flex flex-wrap items-center justify-center gap-2">
    <span class="mr-1 text-sm text-muted">押すと翻訳が変わります</span>
    {#each LANGS as l (l.value)}
      <button
        type="button"
        aria-pressed={lang === l.value}
        onclick={() => (lang = l.value)}
        class="rounded-full border border-line px-4 py-1.5 text-sm font-semibold transition hover:border-ink aria-pressed:border-ink aria-pressed:bg-ink aria-pressed:text-paper"
      >
        {l.label}
      </button>
    {/each}
  </figcaption>
</figure>
