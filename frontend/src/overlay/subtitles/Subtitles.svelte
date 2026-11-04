<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import { TextMorph } from "torph/svelte";
  import { fade, fly } from "svelte/transition";
  import type { Segment } from "../../generated/Segment";
  import type { SubtitleDisplay } from "../../generated/SubtitleDisplay";
  import Rich from "../../lib/Rich.svelte";
  import { api } from "../../lib/api";
  import { loadGlossary } from "../../lib/glossary.svelte";
  import { slide } from "../../lib/slide";
  import { visibleLines } from "../../lib/subtitle-lines";
  import { connect } from "../../lib/ws";

  /** partial が伸びる・final で直るときの文字の動き（DESIGN.md 11 章） */
  const MORPH = { duration: 220, ease: "cubic-bezier(0.19, 1, 0.22, 1)" };

  const segments = new SvelteMap<string, Segment>();
  let now = $state(Date.now());
  /** 見せ方は操作パネルで変え、サーバーから届く（接続直後にも届く） */
  let display = $state<SubtitleDisplay | undefined>();

  /** 画面に初めて出た時刻。待たされている行は、出るまで消えない */
  const shownAt = new Map<string, number>();

  /** 表示時間は翻訳が届いて画面に出てから数える。partial / final の間は消さない */
  const expiresAt = (s: Segment) => {
    const seen = shownAt.get(s.id);
    if (s.settledAt === undefined || seen === undefined || !display) return Infinity;
    const seconds = display.holdSeconds + [...s.text].length * display.perCharSeconds;
    return Math.max(s.settledAt, seen) + seconds * 1000;
  };

  const isLive = (s: Segment) => s.stage !== "discarded" && expiresAt(s) > now;

  const lines = $derived(
    display
      ? visibleLines(
          [...segments.values()].filter(isLive).toSorted((a, b) => a.startedAt - b.startedAt),
          now,
          display,
        )
      : { shown: [], gone: [] },
  );
  const visible = $derived(lines.shown);

  function forget(id: string) {
    segments.delete(id);
    shownAt.delete(id);
  }

  onMount(() => {
    // OBS の中で開かれたか（はじめかたのチェックに使う）。OBS のドックの操作パネルのプレビューも
    // obsstudio を持つかもしれないので、iframe の中は除く
    if ("obsstudio" in window && window.parent === window)
      void api.updateSettings({ obsSeen: true }).catch(() => {});
    const tick = setInterval(() => {
      now = Date.now();
      for (const s of lines.shown) if (!shownAt.has(s.id)) shownAt.set(s.id, now);
      for (const s of lines.gone) forget(s.id);
      for (const [id, s] of segments) if (!isLive(s)) forget(id);
    }, 250);
    const disconnect = connect(["subtitle", "glossary", "display"], (msg) => {
      switch (msg.type) {
        case "snapshot":
          segments.clear();
          for (const s of msg.segments) segments.set(s.id, s);
          break;
        case "segment.upsert":
          segments.set(msg.segment.id, msg.segment);
          break;
        // 辞書ができたとき（と接続直後）に届く
        case "glossary.updated":
          void loadGlossary();
          break;
        case "display":
          display = msg.subtitles;
          break;
      }
    });
    return () => {
      clearInterval(tick);
      disconnect();
    };
  });
</script>

<!-- 見せ方は操作パネルから届く値を CSS 変数で当てる（操作パネルの値が優先。OBS のカスタム CSS で
     大きさを変えるときは .captions に !important で書く。色は :root の変数を上書きする） -->
<ol
  class="captions"
  data-align={display?.align ?? "center"}
  data-background={display?.background ?? "outline"}
  style:--source-size={display && `${display.fontSize}px`}
  style:--translation-size={display && `${display.translationFontSize}px`}
>
  {#each visible as s (s.id)}
    <li
      class="caption {s.stage}"
      animate:slide={{ duration: 200 }}
      in:fly={{ y: 16, duration: 200 }}
      out:fade={{ duration: 400 }}
    >
      <p class="source" lang={s.lang}>
        <!-- partial / final は文字単位で変化させ、enriched ではふりがなとアイコンを足す -->
        {#if s.enriched}<span class="enriched"
            ><Rich tokens={s.enriched.source} lang={s.lang} furigana={display?.furigana} /></span
          >{:else}<TextMorph text={s.text} locale={s.lang} as="span" {...MORPH} />{/if}
      </p>
      {#if s.enriched}
        {#each Object.entries(s.enriched.translations) as [lang, tokens] (lang)}
          <p class="translation" {lang} in:fly={{ y: 8, duration: 250 }}>
            <Rich {tokens} {lang} furigana={display?.furigana} />
          </p>
        {/each}
      {/if}
    </li>
  {/each}
</ol>

<style>
  /* OBS の「カスタム CSS」で上書きできるよう、見た目は変数にまとめる */
  :global(:root) {
    --source-size: 40px;
    --translation-size: 26px;
    --text-color: #fff;
    --hero-color: #ff7ab6;
    --ability-color: #8fd8ff;
    --pending-color: #c8c8c8;
    --outline-color: rgb(0 0 0 / 0.85);
    --gap: 12px;
  }
  :global(html, body) {
    margin: 0;
    background: transparent;
    overflow: hidden;
  }
  /* オーバーレイは字幕の箱（置く場所は OBS で決める）。下から積み上げ、箱の幅で折り返す */
  .captions {
    position: fixed;
    inset: auto 0 0 0;
    margin: 0;
    padding: 0 12px 8px;
    list-style: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--gap);
    text-align: center;
    color: var(--text-color);
  }
  /* 半透明の帯: 明るい場面でも読めるよう、行ごとに黒い帯を敷いて縁取りを細くする */
  .captions[data-background="band"] p {
    -webkit-text-stroke-width: 0.08em;
    text-shadow: none;
    padding: 0.08em 0.45em;
    border-radius: 0.2em;
    background: rgb(0 0 0 / 0.62);
  }
  /* 帯は行の幅だけにする（揃えは .captions の align-items に合わせる） */
  .caption {
    display: flex;
    flex-direction: column;
    align-items: inherit;
  }
  .captions[data-align="left"] {
    align-items: flex-start;
    text-align: left;
  }
  .captions[data-align="right"] {
    align-items: flex-end;
    text-align: right;
  }
  /* 縁取りの太さは行ごとの文字の大きさに合わせる（em は要素自身の font-size で決まる） */
  p {
    margin: 0;
    line-height: 1.5;
    font-weight: 700;
    -webkit-text-stroke: 0.18em var(--outline-color);
    paint-order: stroke fill;
    text-shadow: 0 0.05em 0.15em rgb(0 0 0 / 0.6);
  }
  .source {
    font-size: var(--source-size);
  }
  .translation {
    font-size: var(--translation-size);
  }
  /* 確定前（partial / final）は色を落とし、enriched で白くなる */
  .source {
    transition: color 300ms ease-out;
  }
  .partial .source,
  .final .source {
    color: var(--pending-color);
  }
  /* enriched になったら、ふりがな・アイコン・用語の色を後から重ねる（文字の位置は変えない） */
  .enriched :global(rt),
  .enriched :global(.icon) {
    animation: reveal 300ms ease-out;
  }
  .enriched :global(.entity) {
    animation: tint 400ms ease-out;
  }
  @keyframes reveal {
    from {
      opacity: 0;
    }
  }
  @keyframes tint {
    from {
      color: var(--text-color);
    }
  }
</style>
