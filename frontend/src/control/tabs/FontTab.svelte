<script lang="ts">
  import { onMount } from "svelte";
  import PlayIcon from "@lucide/svelte/icons/play";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { STANDARD } from "../../lib/font-samples";
  import { SUBTITLE_SIZE } from "../../lib/overlay-size";
  import { applyOverlayFonts, fontSample } from "../../lib/overlay-fonts";
  import FontPicker from "../parts/FontPicker.svelte";
  import OverlayPreview from "../parts/OverlayPreview.svelte";
  import { panel } from "../panel.svelte";

  const s = $derived(panel.settings!);
  /** 字幕とランクに出る言語だけを並べる */
  const languages = $derived([
    ...new Set([s.micLanguage, ...s.translateTo, s.rankLanguage ?? s.micLanguage]),
  ]);

  onMount(() => void panel.loadFonts());
  // 見本もオーバーレイと同じく lang.css の --font で描く
  $effect(() => applyOverlayFonts(s.overlayFonts));
</script>

<Card.Root>
  <Card.Header>
    <Card.Title>フォント</Card.Title>
    <Card.Description>
      字幕とランクに共通です。Google Fonts のフォントと、この PC
      に入っているフォントから言語ごとに選べます。アイテム説明のカードはゲームと同じ見た目にするので変わりません
    </Card.Description>
    <Card.Action>
      <Button variant="outline" size="sm" onclick={panel.demo}>
        <PlayIcon />テスト字幕を流す
      </Button>
    </Card.Action>
  </Card.Header>
  <Card.Content class="flex flex-col gap-6">
    <OverlayPreview
      src="/overlay/subtitles.html"
      title="フォントのプレビュー"
      size={SUBTITLE_SIZE}
    />

    <Field.Group>
      {#each languages as language (language)}
        <div class="flex items-center gap-4">
          <div class="min-w-0 flex-1">
            <p class="flex gap-2 text-sm">
              <span class="shrink-0 font-medium">{panel.label(language)}</span>
              <span class="truncate text-muted-foreground">
                {s.overlayFonts[language] || STANDARD}
              </span>
            </p>
            <p lang={language} class="truncate text-lg font-bold">{fontSample(language)}</p>
          </div>
          <FontPicker
            {language}
            label={panel.label(language)}
            value={s.overlayFonts[language]}
            families={panel.fonts}
            onopen={panel.loadFonts}
            onchange={(family) => panel.update({ overlayFonts: { [language]: family } })}
          />
        </div>
      {/each}
    </Field.Group>
  </Card.Content>
</Card.Root>
