<script lang="ts">
  import type { RubySegment } from "../generated/RubySegment";
  import { entityName, glossary } from "./glossary.svelte";
  import Ruby from "./Ruby.svelte";

  let {
    ref,
    spoken,
    ruby: spokenRuby,
    lang,
    furigana,
  }: {
    ref: string;
    /** 話した言葉（原文）。あればそのまま出す。翻訳では無く、公式名を出す */
    spoken?: string;
    /** 話した言葉のふりがな */
    ruby?: RubySegment[];
    lang: string;
    furigana: boolean;
  } = $props();

  const entity = $derived(glossary.entity(ref));
  const label = $derived(spoken ?? (entity ? entityName(entity, lang) : ref));
  // 公式名どおりなら、名前ごとに確かめた辞書の読み（造語に強い）。別名なら文の読み
  const ruby = $derived.by(() => {
    if (lang !== "ja" || !furigana) return undefined;
    if (entity?.ruby && label === entityName(entity, "ja")) return entity.ruby;
    return spokenRuby;
  });
</script>

<!-- 辞書からアイコンと色（と翻訳での公式名）を引く。辞書にない（まだ読み込んでいない）ときは文字だけ -->
{#if entity}<span class="entity {entity.kind} {entity.slot ?? ''}"
    >{#if entity.icon}<img
        class="icon"
        src={entity.icon}
        alt=""
      />{/if}{#if ruby}{#each ruby as seg, i (i)}<Ruby
          text={seg.text}
          ruby={seg.ruby}
        />{/each}{:else}{label}{/if}</span
  >{:else}<span class="entity">{label}</span>{/if}

<style>
  /* 種類ごとの色。アイテムはゲーム内と同じくスロット（武器 / バイタリティ / スピリット）で分ける */
  .entity {
    color: var(--entity-color, #ffd34d);
    white-space: nowrap;
  }
  .hero {
    color: var(--hero-color, #ffd34d);
  }
  .ability {
    color: var(--ability-color, #8fd8ff);
  }
  .weapon {
    color: var(--weapon-color, #ffb35c);
  }
  .vitality {
    color: var(--vitality-color, #98e36b);
  }
  .spirit {
    color: var(--spirit-color, #cfa2ff);
  }
  .icon {
    height: 1.1em;
    /* アイテムのアイコンは色付きの四角なので、前の文字にくっつかないよう左右に空ける */
    margin: 0 0.16em 0 0.2em;
    vertical-align: -0.18em;
    border-radius: 0.18em;
    /* 文字の縁取り（-webkit-text-stroke）がかからないので影で浮かせる */
    filter: drop-shadow(0 0 0.06em rgb(0 0 0 / 0.9));
  }
</style>
