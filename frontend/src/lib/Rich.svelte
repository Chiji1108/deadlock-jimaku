<script lang="ts">
  import type { Token } from "../generated/Token";

  let { tokens, furigana = true }: { tokens: Token[]; furigana?: boolean } = $props();
</script>

<!-- 日本語ではトークンの間に空白が入らないよう、ブロックの間に改行を入れない -->
{#each tokens as token, i (i)}{#if token.t === "entity"}<span class="entity"
      >{token.spoken ?? token.ref}</span
    >{:else if token.ruby && furigana}<ruby>{token.text}<rt>{token.ruby}</rt></ruby
    >{:else}{token.text}{/if}{/each}

<style>
  /* Phase 1 で辞書の公式名とアイコンに置き換える */
  .entity {
    color: var(--entity-color, #ffd34d);
  }
  /* ルビが親文字より長くても、親文字の間を空けない */
  ruby {
    ruby-align: center;
  }
  rt {
    font-size: 0.5em;
  }
</style>
