<script lang="ts">
  // 一番上の配信画面の見本。アプリの字幕とランクの部品に、用意した台詞を流す
  import { onMount } from "svelte";
  import Captions from "../../../frontend/src/lib/Captions.svelte";
  import RankBadge from "../../../frontend/src/lib/RankBadge.svelte";
  import { defineCards, type Cards } from "../lib/cards";
  import { display, enriched, LANGS, LINES, partial, RANK, spokenText } from "../lib/demo";

  let { cards }: { cards: Cards } = $props();

  /** 配信画面の幅。部品の大きさは OBS の 1920px の画面に置いたときの比で決める */
  const SCREEN = 1920;
  const CARD_WIDTH = 450;

  let stream: HTMLElement;
  let width = $state(1152);
  let lang = $state<(typeof LANGS)[number]["value"]>("en");
  let index = $state(0);
  let text = $state<string | undefined>();
  let cycle = $state(0);
  let card = $state(LINES[0]!.card);
  let cardShown = $state(false);
  let cardsReady = $state(false);
  let inView = true;
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
    void defineCards().then(() => (cardsReady = true));

    if (matchMedia("(prefers-reduced-motion: reduce)").matches) {
      text = spokenText(line);
      cardShown = true;
      return;
    }
    // 画面の外にいる間は字幕を進めない
    const observer = new IntersectionObserver(([entry]) => {
      inView = entry?.isIntersecting ?? false;
    });
    observer.observe(stream);
    let stopped = false;
    void (async () => {
      for (let i = 0; !stopped; i = (i + 1) % LINES.length) {
        while (!inView && !stopped) await sleep(300);
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
        cardShown = true;
        await sleep(5000);
        if (LINES[(i + 1) % LINES.length]!.card !== card) cardShown = false;
        text = undefined;
        await sleep(600);
      }
    })();
    return () => {
      stopped = true;
      observer.disconnect();
    };
  });
</script>

<figure class="hero-stage mx-auto mt-8">
  <div class="stream" class:narrow bind:this={stream} bind:clientWidth={width}>
    <video
      data-play-in-view
      src="/media/hero.mp4"
      poster="/media/hero.webp"
      muted
      loop
      playsinline
      aria-label="Deadlock の配信画面。左上にランク、右にアイテム説明、下の中央に字幕が重なる見本"
    ></video>
    <div class="stream-rank" style:--scale={scale}>
      <RankBadge rank={RANK} lang="ja" content="gauge" />
    </div>
    <dl-provider language="japanese">
      <div class="stream-card" class:show={cardShown} style:zoom={scale * (narrow ? 0.9 : 1)}>
        {#if cardsReady}
          <dl-item-tooltip
            itemData={cards[card]?.item}
            componentItemsData={cards[card]?.components}
            parentItemsData={cards[card]?.parents}
          ></dl-item-tooltip>
        {/if}
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
