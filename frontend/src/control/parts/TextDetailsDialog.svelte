<script lang="ts">
  import SlidersHorizontalIcon from "@lucide/svelte/icons/sliders-horizontal";
  import { Button } from "$lib/components/ui/button";
  import * as Dialog from "$lib/components/ui/dialog";
  import * as Field from "$lib/components/ui/field";
  import { Input } from "$lib/components/ui/input";
  import type { SubtitleDisplayPatch } from "../../generated/SubtitleDisplayPatch";
  import { panel } from "../panel.svelte";

  // 文字の大きさと色を数値で決める。並べた選択肢で足りない人向け（設定の範囲はサーバーと同じ）
  const SIZE = { min: 16, max: 96 };
  const HEX = /^#[0-9a-fA-F]{6}$/;

  const d = $derived(panel.settings!.subtitleDisplay);
  let open = $state(false);
  let fontSize = $state(0);
  let translationFontSize = $state(0);
  let textColor = $state("");
  /** 開いたときの値 */
  let opened = { fontSize: 0, translationFontSize: 0, textColor: "" };

  /** 開くたびに、今の設定から始める */
  function reset() {
    opened = { ...d };
    ({ fontSize, translationFontSize, textColor } = d);
  }

  const valid = $derived(
    [fontSize, translationFontSize].every(
      (n) => Number.isInteger(n) && n >= SIZE.min && n <= SIZE.max,
    ) && HEX.test(textColor),
  );

  async function save(e: SubmitEvent) {
    e.preventDefault();
    // 変えた項目だけを送る（開いている間にほかで変えた項目を、開いたときの値で戻さない）
    const patch: SubtitleDisplayPatch = {};
    if (fontSize !== opened.fontSize) patch.fontSize = fontSize;
    if (translationFontSize !== opened.translationFontSize)
      patch.translationFontSize = translationFontSize;
    if (textColor !== opened.textColor) patch.textColor = textColor;
    if (await panel.update({ subtitleDisplay: patch })) open = false;
  }
</script>

<Dialog.Root bind:open onOpenChange={(o) => o && reset()}>
  <Dialog.Trigger>
    {#snippet child({ props })}
      <Button variant="ghost" size="sm" {...props}><SlidersHorizontalIcon />数値で指定</Button>
    {/snippet}
  </Dialog.Trigger>
  <Dialog.Content class="sm:max-w-md">
    <form onsubmit={save} class="flex flex-col gap-6">
      <Dialog.Header>
        <Dialog.Title>文字の大きさと色</Dialog.Title>
        <Dialog.Description>
          大きさは OBS のブラウザソースの中での px です（{SIZE.min}〜{SIZE.max}）
        </Dialog.Description>
      </Dialog.Header>
      <Field.Group>
        <Field.Field orientation="horizontal">
          <Field.Label for="font-size" class="flex-1">話した言葉の大きさ</Field.Label>
          <Input
            id="font-size"
            type="number"
            class="w-24"
            min={SIZE.min}
            max={SIZE.max}
            bind:value={fontSize}
          />
        </Field.Field>
        <Field.Field orientation="horizontal">
          <Field.Label for="translation-font-size" class="flex-1">翻訳の大きさ</Field.Label>
          <Input
            id="translation-font-size"
            type="number"
            class="w-24"
            min={SIZE.min}
            max={SIZE.max}
            bind:value={translationFontSize}
          />
        </Field.Field>
        <Field.Field orientation="horizontal">
          <Field.Label for="text-color" class="flex-1">文字の色</Field.Label>
          <input
            type="color"
            class="size-9 cursor-pointer rounded-md border-0 bg-transparent p-0"
            aria-label="色を選ぶ"
            value={HEX.test(textColor) ? textColor : d.textColor}
            oninput={(e) => (textColor = e.currentTarget.value)}
          />
          <Input id="text-color" class="w-28 font-mono" bind:value={textColor} />
        </Field.Field>
      </Field.Group>
      <Dialog.Footer>
        <Dialog.Close>
          {#snippet child({ props })}
            <Button variant="outline" {...props}>やめる</Button>
          {/snippet}
        </Dialog.Close>
        <Button type="submit" disabled={!valid}>保存</Button>
      </Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
