<script lang="ts">
  import type { Token } from "../generated/Token";
  import EntityRef from "./EntityRef.svelte";
  import Ruby from "./Ruby.svelte";

  let {
    tokens,
    lang,
    furigana = true,
  }: { tokens: Token[]; lang: string; furigana?: boolean } = $props();
</script>

<!-- 日本語ではトークンの間に空白が入らないよう、ブロックの間に改行を入れない -->
{#each tokens as token, i (i)}{#if token.t === "entity"}<EntityRef
      ref={token.ref}
      spoken={token.spoken}
      ruby={token.ruby}
      {lang}
      {furigana}
    />{:else}<Ruby text={token.text} ruby={furigana ? token.ruby : undefined} />{/if}{/each}
