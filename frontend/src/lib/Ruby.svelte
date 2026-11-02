<script lang="ts">
  let { text, ruby }: { text: string; ruby?: string } = $props();
</script>

{#if ruby}<ruby>{text}<rt>{ruby}</rt></ruby>{:else}{text}{/if}

<style>
  /* ルビが親文字より長くても、親文字の間を空けない */
  ruby {
    ruby-align: center;
  }
  rt {
    font-size: 0.5em;
  }
</style>
