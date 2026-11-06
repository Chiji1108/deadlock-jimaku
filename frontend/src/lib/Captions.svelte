<script lang="ts">
  // 字幕の見た目。OBS のオーバーレイと LP の見本で同じものを使う。親の下に揃えて積み上げる
  import { fade, fly } from "svelte/transition";
  import type { Segment } from "../generated/Segment";
  import type { SubtitleDisplay } from "../generated/SubtitleDisplay";
  import GrowingText from "./GrowingText.svelte";
  import Rich from "./Rich.svelte";
  import { slide } from "./slide";
  import { draftTerms, showsSource, translationsOf } from "./subtitle-lines";

  let { segments, display }: { segments: Segment[]; display?: SubtitleDisplay } = $props();

  /** 1 行だけのとき、次の字幕に入れ替わるなら、古い行をフェードさせない（2 行に見える） */
  const replacing = () => display?.maxLines === 1 && segments.length > 0;
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
  {#each segments as s (s.id)}
    {@const known = draftTerms(s)}
    <li
      class="caption {s.stage}"
      animate:slide={{ duration: 200 }}
      in:fly={{ y: 16, duration: 200 }}
      out:fade={{ duration: replacing() ? 0 : 400 }}
    >
      {#if showsSource(s, display)}
        <p class="source" lang={s.lang}>
          <!-- partial / final は増えた文字だけをフェードさせ、enriched ではふりがなとアイコンを足す。
               途中から付いていたものは、出し直して見えないよう足し直さない。確定で初めて見つかった
               用語にだけ演出を付ける -->
          {#if s.enriched}<span class:fresh={!s.draftSource}
              ><Rich
                tokens={s.enriched.source}
                lang={s.lang}
                furigana={display?.furigana}
                {known}
              /></span
            >{:else}<GrowingText
              text={s.text}
              tokens={s.draftSource}
              furigana={display?.furigana}
            />{/if}
        </p>
      {/if}
      {#each Object.entries(translationsOf(s)) as [lang, tokens] (lang)}
        <p class="translation" {lang} in:fly={{ y: 8, duration: 250 }}>
          <Rich
            tokens={tokens ?? []}
            {lang}
            furigana={display?.furigana}
            known={s.enriched && known}
          />
        </p>
      {/each}
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
  /* オーバーレイは字幕の箱（置く場所は OBS で決める）。下から積み上げ、箱の幅で折り返す */
  .captions {
    position: absolute;
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
    max-width: 100%;
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
    /* 文節で折り返す。ふりがなの付いた文は長い 1 文節に見えることがあるので、箱に収まらなければ途中でも折る */
    word-break: auto-phrase;
    overflow-wrap: anywhere;
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
  .caption :global(.fresh rt),
  .caption :global(.fresh .icon) {
    animation: reveal 300ms ease-out;
  }
  .caption :global(.fresh .entity),
  .caption :global(.entity.fresh) {
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
