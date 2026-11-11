<script lang="ts">
  // モードを詳しく選ぶ。工程ごとに作り方を変える（DESIGN.md 9.6）。押したらすぐ保存する
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "$lib/components/ui/button";
  import * as Field from "$lib/components/ui/field";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import { panel } from "../panel.svelte";
  import {
    ITEMS,
    anyPaid,
    chromeFinals,
    keyNames,
    llmTranslation,
    providers,
    unavailable,
    usesChrome,
  } from "../../lib/engines";
  import { translationOnly } from "../../lib/subtitle-lines";
  import type { Engines } from "../../generated/Engines";

  const s = $derived(panel.settings!);
  const t = $derived(s.engines);
  const items = Object.entries(ITEMS) as [keyof Engines, (typeof ITEMS)[keyof Engines]][];
  const choose = <K extends keyof Engines>(item: K, value: Engines[K]) =>
    panel.update({ engines: { ...t, [item]: value } });
</script>

<Dialog.Root>
  <Dialog.Trigger>
    {#snippet child({ props })}
      <Button {...props} variant="outline" size="sm" class="w-fit">詳しく選ぶ</Button>
    {/snippet}
  </Dialog.Trigger>
  <Dialog.Content class="max-h-[90vh] overflow-y-auto sm:max-w-xl">
    <Dialog.Header>
      <Dialog.Title>字幕の作り方を詳しく選ぶ</Dialog.Title>
      <Dialog.Description>
        工程ごとに、無料か有料かを選べます。料金はそれぞれの会社の単価です。切り替えると、字幕は始め直します
      </Dialog.Description>
    </Dialog.Header>
    <div class="flex flex-col gap-5">
      {#each items as [item, spec] (item)}
        <Field.Field>
          <Field.Label>{spec.label}</Field.Label>
          <Field.Description>
            {spec.hint}{#if item === "live" && translationOnly(s) && t.live === "gptLiveTranscribe"}。翻訳だけを出しているので、途中の字幕は出ません{/if}
          </Field.Description>
          <ToggleGroup.Root
            type="single"
            variant="outline"
            spacing={2}
            class="flex-wrap justify-start"
            value={t[item]}
            onValueChange={(v) => v && choose(item, v as Engines[typeof item])}
          >
            {#each spec.options as o (o.value)}
              <ToggleGroup.Item
                value={o.value}
                class="h-auto flex-col items-start gap-0 py-2"
                disabled={unavailable(t, item, o.value)}
              >
                <span>{o.label}</span>
                <span class="text-xs text-muted-foreground tabular-nums">{o.price}</span>
                {#if o.note}<span class="text-xs whitespace-normal text-muted-foreground"
                    >{o.note}</span
                  >{/if}
              </ToggleGroup.Item>
            {/each}
          </ToggleGroup.Root>
          {#if item === "live" && chromeFinals(t)}
            <Field.Description
              >確定の文字起こしが Chrome なら、途中の字幕も Chrome です</Field.Description
            >
          {:else if item === "live" && t.translation === "chromeTranslator"}
            <Field.Description
              >Chrome の翻訳を使っている間は、途中の字幕も Chrome です</Field.Description
            >
          {:else if item === "terms" && llmTranslation(t)}
            <Field.Description
              >翻訳が GPT なので、用語は GPT が翻訳と同時に判定します</Field.Description
            >
          {:else if item === "translation" && !usesChrome(t)}
            <Field.Description
              >Chrome の翻訳は、途中の字幕が Chrome のときだけ選べます</Field.Description
            >
          {/if}
        </Field.Field>
      {/each}
      {#if anyPaid(t)}
        <p class="text-sm text-muted-foreground">
          この作り方には {keyNames(
            providers(t),
          )}が要ります。使った分の目安は「料金」タブに工程ごとに出ます
        </p>
      {/if}
    </div>
  </Dialog.Content>
</Dialog.Root>
