<script lang="ts">
  // 節の中の字幕の見本
  import Captions from "../../../frontend/src/lib/Captions.svelte";
  import { display, enriched, LINES } from "../lib/demo";

  let { stage, line }: { stage: "enriched" | "translated"; line: number } = $props();

  const segment = $derived(
    enriched(stage, LINES[line]!, stage === "translated" ? "en" : undefined),
  );
</script>

<Captions segments={[segment]} display={display(21, 15)} />
