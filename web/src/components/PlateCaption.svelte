<script lang="ts">
  // 字幕の段階を並べて見せる見本
  import Captions from "../../../frontend/src/lib/Captions.svelte";
  import { display, enriched, LINES, partial, spokenText } from "../lib/demo";

  let { stage }: { stage: "partial" | "enriched" | "translated" } = $props();

  const line = LINES[0]!;
  const segment = $derived(
    stage === "partial"
      ? partial(stage, spokenText(line).slice(0, 12))
      : enriched(stage, line, stage === "translated" ? "en" : undefined),
  );
</script>

<Captions segments={[segment]} display={display(21, 15)} />
