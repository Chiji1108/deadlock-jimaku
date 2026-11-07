<script lang="ts">
  import type { Snippet } from "svelte";
  import ChevronRightIcon from "@lucide/svelte/icons/chevron-right";
  import { Button } from "$lib/components/ui/button";
  import * as Field from "$lib/components/ui/field";
  import { Switch } from "$lib/components/ui/switch";
  import type { TextLook } from "../../generated/TextLook";
  import {
    outlineStandard,
    shadowOn,
    type MergePatch,
    type TextStandard,
  } from "../../lib/text-style";
  import ColorField from "./ColorField.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import StyleNumber from "./StyleNumber.svelte";

  // 字幕とランクで共通の「文字の見た目」。基本の項目を見せ、残りは「詳しく」に畳む。
  // 変えると表示中のオーバーレイとプレビューにそのまま届く
  let {
    look,
    standard,
    bandHint,
    onchange,
    onreset,
    fonts,
    details,
  }: {
    look: TextLook;
    standard: TextStandard;
    bandHint: string;
    onchange: (patch: MergePatch<TextLook>) => Promise<boolean>;
    onreset: () => Promise<boolean>;
    /** フォントの行 */
    fonts: Snippet;
    /** 「詳しく」の行のうち、字幕とランクで違うもの */
    details: Snippet;
  } = $props();

  let confirming = $state(false);
  const OUTLINE_RANGE = { min: 0, max: 50, step: 1 };
  const BLUR_RANGE = { min: 0, max: 100, step: 1 };
  const outlineWidth = $derived(outlineStandard(look, standard));
</script>

<div class="flex flex-col gap-6">
  <Field.Field>
    <Field.Label>文字の見た目</Field.Label>
    {@render fonts()}
    <Field.Description>フォントにない太さは、太字を合成して描きます</Field.Description>
  </Field.Field>

  <ColorField
    label="文字の色"
    description="ヒーロー名、スキル名、アイテム名の色分けはそのままです"
    value={look.color}
    standard={standard.color}
    onchange={(color) => onchange({ color })}
  />

  <StyleNumber
    label="縁取りの幅"
    value={look.outline.width}
    standard={outlineWidth}
    presets={[0, 0.08, standard.outlineWidth, 0.3]}
    percent
    range={OUTLINE_RANGE}
    onchange={(width) => onchange({ outline: { width } })}
  />
  <ColorField
    label="縁取りの色"
    value={look.outline.color}
    standard={standard.outlineColor}
    onchange={(color) => onchange({ outline: { color } })}
  />

  <Field.Field orientation="horizontal">
    <Field.Content>
      <Field.Label>半透明の帯</Field.Label>
      <Field.Description>{bandHint}</Field.Description>
    </Field.Content>
    <Switch
      aria-label="半透明の帯"
      checked={look.background.enabled}
      onCheckedChange={(enabled) => onchange({ background: { enabled } })}
    />
  </Field.Field>

  <details class="group flex flex-col">
    <summary
      class="flex cursor-pointer list-none items-center gap-1 text-sm font-medium select-none"
    >
      <ChevronRightIcon class="size-4 transition-transform group-open:rotate-90" />詳しく
    </summary>
    <div class="flex flex-col gap-6 pt-4">
      <Field.Field orientation="horizontal">
        <Field.Label class="flex-1">影</Field.Label>
        <Switch
          aria-label="影"
          checked={shadowOn(look)}
          onCheckedChange={(enabled) => onchange({ shadow: { enabled } })}
        />
      </Field.Field>
      {#if shadowOn(look)}
        <StyleNumber
          label="影のぼかし"
          value={look.shadow.blur}
          standard={standard.shadowBlur}
          presets={[0, 0.1, 0.3, 0.6]}
          percent
          range={BLUR_RANGE}
          onchange={(blur) => onchange({ shadow: { blur } })}
        />
        <ColorField
          label="影の色"
          value={look.shadow.color}
          standard={standard.shadowColor}
          onchange={(color) => onchange({ shadow: { color } })}
        />
      {/if}
      {@render details()}
      {#if look.background.enabled}
        <ColorField
          label="帯の色"
          value={look.background.color}
          standard={standard.bandColor}
          onchange={(color) => onchange({ background: { color } })}
        />
        <StyleNumber
          label="帯の濃さ"
          value={look.background.opacity}
          standard={standard.bandOpacity}
          presets={[0.25, 0.4, 0.8, 1]}
          percent
          range={{ min: 0, max: 100, step: 1 }}
          onchange={(opacity) => onchange({ background: { opacity } })}
        />
      {/if}
      <div>
        <Button variant="outline" size="sm" onclick={() => (confirming = true)}>
          文字の見た目を標準に戻す
        </Button>
      </div>
    </div>
  </details>
</div>

<ConfirmDialog
  bind:open={confirming}
  title="文字の見た目を標準に戻しますか"
  action="標準に戻す"
  onconfirm={onreset}
>
  <p>フォント、色、縁取り、影などの設定を全部消します。</p>
</ConfirmDialog>
