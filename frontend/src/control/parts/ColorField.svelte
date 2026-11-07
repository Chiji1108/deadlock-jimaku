<script lang="ts">
  import { Button } from "$lib/components/ui/button";
  import * as Field from "$lib/components/ui/field";

  // 色の 1 行。動かしている間も順に送ってプレビューに出す。応答を待つ間の色はまとめて、最後の色だけ送る。
  // 「標準に戻す」も同じ列で送るので、待っている色に追い越されない
  let {
    label,
    description,
    value,
    standard,
    onchange,
  }: {
    label: string;
    description?: string;
    value: string | null;
    standard: string;
    /** null は標準に戻す */
    onchange: (color: string | null) => Promise<unknown>;
  } = $props();

  /** 送っている色。undefined なら送っておらず、null は標準 */
  let moving = $state<string | null>();
  let next: string | null | undefined;
  const shown = $derived(moving === undefined ? value : moving);

  async function send(color: string | null) {
    const idle = moving === undefined;
    moving = next = color;
    if (!idle) return;
    while (next !== undefined) {
      const sending = next;
      next = undefined;
      await onchange(sending);
    }
    moving = undefined;
  }
</script>

<Field.Field orientation="horizontal">
  <Field.Content>
    <Field.Label>{label}</Field.Label>
    {#if description}<Field.Description>{description}</Field.Description>{/if}
  </Field.Content>
  {#if shown}
    <Button variant="ghost" size="sm" onclick={() => send(null)}>標準に戻す</Button>
  {/if}
  <input
    type="color"
    aria-label={label}
    class="h-8 w-12 shrink-0 cursor-pointer rounded-md border bg-transparent p-0.5"
    value={shown ?? standard}
    oninput={(e) => send(e.currentTarget.value)}
  />
</Field.Field>
