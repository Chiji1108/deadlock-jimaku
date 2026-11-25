<script lang="ts">
  // 見出しや項目名の横の「?」。仕組みや細かい決まりなど、気になった人だけが読む説明を、押したときに出す
  import type { Snippet } from "svelte";
  import CircleQuestionMarkIcon from "@lucide/svelte/icons/circle-question-mark";
  import * as Popover from "$lib/components/ui/popover";

  let { children }: { children: Snippet } = $props();
</script>

<Popover.Root>
  <Popover.Trigger
    aria-label="説明"
    class="inline-flex size-5 shrink-0 items-center justify-center rounded-full align-middle text-muted-foreground hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
  >
    <CircleQuestionMarkIcon class="size-4" />
  </Popover.Trigger>
  <Popover.Content align="start" class="w-80 gap-2 font-normal text-muted-foreground">
    {@render children()}
  </Popover.Content>
</Popover.Root>
