<script lang="ts">
  import PlayIcon from "@lucide/svelte/icons/play";
  import AlignCenterIcon from "@lucide/svelte/icons/text-align-center";
  import AlignEndIcon from "@lucide/svelte/icons/text-align-end";
  import AlignStartIcon from "@lucide/svelte/icons/text-align-start";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { Switch } from "$lib/components/ui/switch";
  import * as ToggleGroup from "$lib/components/ui/toggle-group";
  import type { Align } from "../../generated/Align";
  import type { SubtitleDisplayPatch } from "../../generated/SubtitleDisplayPatch";
  import type { MergePatch } from "../../lib/text-style";
  import { cleared, LINE_HEIGHT, SUBTITLE_TEXT } from "../../lib/text-style";
  import type { SubtitleStyle } from "../../generated/SubtitleStyle";
  import AddToObs from "../parts/AddToObs.svelte";
  import ColorField from "../parts/ColorField.svelte";
  import OverlayPreview from "../parts/OverlayPreview.svelte";
  import DisplayTimingDialog from "../parts/DisplayTimingDialog.svelte";
  import FontPicker from "../parts/FontPicker.svelte";
  import FontSizeDialog from "../parts/FontSizeDialog.svelte";
  import StyleNumber from "../parts/StyleNumber.svelte";
  import TextStyleFields from "../parts/TextStyleFields.svelte";
  import { SUBTITLE_SIZE } from "../../lib/overlay-size";
  import { overlayUrl, panel } from "../panel.svelte";
  import { translationOnly } from "../../lib/subtitle-lines";

  const ALIGNS: { value: Align; label: string; icon: typeof AlignStartIcon }[] = [
    { value: "left", label: "左寄せ", icon: AlignStartIcon },
    { value: "center", label: "中央", icon: AlignCenterIcon },
    { value: "right", label: "右寄せ", icon: AlignEndIcon },
  ];
  /** 文字の大きさ（px）。もっと大きくしたいときは「数値で指定」 */
  const SIZE_PRESETS = [12, 16, 20, 26, 32, 40];
  const SIZES = [
    { field: "fontSize", label: "話した言葉の大きさ" },
    { field: "translationFontSize", label: "翻訳の大きさ" },
  ] as const;
  /** 同時に出す字幕の数。0 は制限なし */
  const LINES = [
    { value: 1, label: "1" },
    { value: 2, label: "2" },
    { value: 3, label: "3" },
    { value: 0, label: "制限なし" },
  ];
  const HOLDS = [2, 4, 6, 8];

  const s = $derived(panel.settings!);
  const d = $derived(s.subtitleDisplay);
  const noTargets = $derived(s.translateTo.length === 0);
  const languages = $derived([...new Set([s.micLanguage, ...s.translateTo])]);
  const onlyTranslation = $derived(translationOnly(s));
  const bg = $derived(s.subtitleStyle.background);
  /** 変えた項目だけを送る（続けて変えても、前の変更を古い値で戻さない） */
  const set = (patch: SubtitleDisplayPatch) => panel.update({ subtitleDisplay: patch });
  const setStyle = (patch: MergePatch<SubtitleStyle>) => panel.update({ subtitleStyle: patch });
</script>

<div class="flex flex-col gap-6">
  <Card.Root>
    <Card.Header>
      <Card.Title>字幕オーバーレイ</Card.Title>
      <Card.Description
        >ここで変えた見せ方は、OBS に表示中の字幕にもそのまま反映されます</Card.Description
      >
      <Card.Action>
        <Button variant="outline" size="sm" onclick={panel.demo}>
          <PlayIcon />テスト字幕を流す
        </Button>
      </Card.Action>
    </Card.Header>
    <Card.Content class="flex flex-col gap-6">
      <OverlayPreview
        src="/overlay/subtitles.html?preview={languages.join(',')}"
        title="字幕オーバーレイのプレビュー"
        size={SUBTITLE_SIZE}
      />
      <AddToObs
        url={overlayUrl("subtitles")}
        name="字幕"
        size={SUBTITLE_SIZE}
        hint="大きさは、ソースを引き伸ばさずにプロパティの幅と高さで変えてください"
      />

      <Field.Group>
        <Field.Field>
          <Field.Label>揃え</Field.Label>
          <Field.Description
            >OBS で字幕を置く位置に合わせます（画面の左に置くなら左寄せ）</Field.Description
          >
          <ToggleGroup.Root
            type="single"
            variant="outline"
            class="w-full"
            value={d.align}
            onValueChange={(v) => v && set({ align: v as Align })}
          >
            {#each ALIGNS as a (a.value)}
              <ToggleGroup.Item value={a.value} class="flex-1"><a.icon />{a.label}</ToggleGroup.Item
              >
            {/each}
          </ToggleGroup.Root>
        </Field.Field>
        {#each SIZES as f (f.field)}
          <Field.Field>
            <div class="flex items-center justify-between">
              <Field.Label>{f.label}</Field.Label>
              <FontSizeDialog field={f.field} label={f.label} />
            </div>
            <!-- 数値で指定して選択肢から外れたら、どれも選ばない -->
            <ToggleGroup.Root
              type="single"
              variant="outline"
              class="w-full"
              value={String(d[f.field])}
              onValueChange={(v) => v && set({ [f.field]: Number(v) })}
            >
              {#each SIZE_PRESETS as px (px)}
                <ToggleGroup.Item value={String(px)} class="flex-1">{px}px</ToggleGroup.Item>
              {/each}
            </ToggleGroup.Root>
            {#if !SIZE_PRESETS.includes(d[f.field])}
              <Field.Description>数値で指定しています（{d[f.field]}px）</Field.Description>
            {/if}
            {#if f.field === "fontSize" && d.fontSize < 16 && d.furigana && !onlyTranslation && s.micLanguage === "ja"}
              <Field.Description>小さくすると、ふりがなが読みにくくなります</Field.Description>
            {/if}
          </Field.Field>
        {/each}

        <Field.Separator />

        <TextStyleFields
          look={s.subtitleStyle}
          standard={SUBTITLE_TEXT}
          colorHint="ヒーロー名、スキル名、アイテム名の色分けは残します"
          onchange={setStyle}
          onreset={() => setStyle(cleared(s.subtitleStyle))}
          detailValues={[
            s.subtitleStyle.lineHeight,
            ...languages.map((language) => s.subtitleStyle.languages[language]?.letterSpacing),
          ]}
        >
          {#snippet fonts()}
            <Field.Description>
              Google Fonts のフォントと、この PC に入っているフォントから言語ごとに選べます
            </Field.Description>
            {#each languages as language (language)}
              <FontPicker
                target={{ kind: "subtitle", language }}
                {language}
                label={panel.label(language)}
                standardWeight={SUBTITLE_TEXT.weight}
              />
            {/each}
          {/snippet}
          {#snippet band()}
            <Field.Field orientation="horizontal">
              <Field.Content>
                <Field.Label>半透明の帯</Field.Label>
                <Field.Description
                  >明るい場面でも読めるよう、行の後ろに帯を敷きます</Field.Description
                >
              </Field.Content>
              <Switch
                aria-label="半透明の帯"
                checked={bg.enabled}
                onCheckedChange={(enabled) => setStyle({ background: { enabled } })}
              />
            </Field.Field>
          {/snippet}
          {#snippet details()}
            <StyleNumber
              label="行間"
              value={s.subtitleStyle.lineHeight}
              standard={LINE_HEIGHT}
              presets={[1.2, 1.5, 1.8, 2]}
              range={{ min: 1, max: 3, step: 0.1 }}
              onchange={(lineHeight) => setStyle({ lineHeight })}
            />
            {#each languages as language (language)}
              <StyleNumber
                label="{panel.label(language)}の字間"
                value={s.subtitleStyle.languages[language]?.letterSpacing ?? null}
                standard={SUBTITLE_TEXT.letterSpacing}
                presets={[0, 0.02, 0.05, 0.1]}
                percent
                range={{ min: -10, max: 50, step: 1 }}
                onchange={(letterSpacing) =>
                  panel.setFontStyle({ kind: "subtitle", language }, { letterSpacing })}
              />
            {/each}
            {#if bg.enabled}
              <ColorField
                label="帯の色"
                value={bg.color}
                standard={SUBTITLE_TEXT.bandColor}
                onchange={(color) => setStyle({ background: { color } })}
              />
              <StyleNumber
                label="帯の濃さ"
                value={bg.opacity}
                standard={SUBTITLE_TEXT.bandOpacity}
                presets={[0.25, 0.4, 0.8, 1]}
                percent
                range={{ min: 0, max: 100, step: 1 }}
                onchange={(opacity) => setStyle({ background: { opacity } })}
              />
            {/if}
          {/snippet}
        </TextStyleFields>

        <Field.Separator />

        <Field.Field orientation="horizontal">
          <Field.Content>
            <Field.Label for="show-source">話した言葉を出す</Field.Label>
            <Field.Description>
              {#if noTargets}
                翻訳先がないときは、話した言葉を出します
              {:else}
                オフにすると翻訳だけを出します。{#if s.speechEngine !== "chrome"}途中の字幕とふりがなの料金がかかりません。{/if}字幕は翻訳が終わってから出ます。切り替えると、字幕は始め直します
              {/if}
            </Field.Description>
          </Field.Content>
          <Switch
            id="show-source"
            checked={!onlyTranslation}
            disabled={noTargets}
            onCheckedChange={(v) => set({ showSource: v })}
          />
        </Field.Field>

        <Field.Field orientation="horizontal">
          <Field.Content>
            <Field.Label for="furigana">ふりがなを付ける</Field.Label>
            <Field.Description
              >漢字に読みを付けます（日本語を学んでいる視聴者向け）</Field.Description
            >
          </Field.Content>
          <Switch
            id="furigana"
            checked={d.furigana}
            disabled={onlyTranslation && !s.translateTo.includes("ja")}
            onCheckedChange={(v) => set({ furigana: v })}
          />
        </Field.Field>

        <Field.Separator />

        <Field.Field>
          <Field.Label>同時に出す字幕の数</Field.Label>
          <ToggleGroup.Root
            type="single"
            variant="outline"
            class="w-full"
            value={String(d.maxLines)}
            onValueChange={(v) => v && set({ maxLines: Number(v) })}
          >
            {#each LINES as n (n.value)}
              <ToggleGroup.Item value={String(n.value)} class="flex-1">{n.label}</ToggleGroup.Item>
            {/each}
          </ToggleGroup.Root>
        </Field.Field>
        <Field.Field>
          <div class="flex items-center justify-between">
            <Field.Label>消すまでの時間</Field.Label>
            <DisplayTimingDialog />
          </div>
          <Field.Description>長い文は、そのぶん長く出します</Field.Description>
          <ToggleGroup.Root
            type="single"
            variant="outline"
            class="w-full"
            value={String(d.holdSeconds)}
            onValueChange={(v) => v && set({ holdSeconds: Number(v) })}
          >
            {#each HOLDS as n (n)}
              <ToggleGroup.Item value={String(n)} class="flex-1">{n} 秒</ToggleGroup.Item>
            {/each}
          </ToggleGroup.Root>
          {#if !HOLDS.includes(d.holdSeconds)}
            <Field.Description>数値で指定しています（{d.holdSeconds} 秒）</Field.Description>
          {/if}
        </Field.Field>
      </Field.Group>
    </Card.Content>
  </Card.Root>
</div>
