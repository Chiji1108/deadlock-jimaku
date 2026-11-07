<script lang="ts">
  import { onMount } from "svelte";
  import PlayIcon from "@lucide/svelte/icons/play";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import { SUBTITLE_SIZE } from "../../lib/overlay-size";
  import { applyOverlayFonts, endonym } from "../../lib/overlay-fonts";
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
      字幕とランクに共通です。この PC
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
        <Field.Field>
          <Field.Label for="font-{language}">{panel.label(language)}</Field.Label>
          <FontPicker
            id="font-{language}"
            {language}
            value={s.overlayFonts[language]}
            families={panel.fonts}
            onopen={panel.loadFonts}
            onchange={(family) => panel.update({ overlayFonts: { [language]: family } })}
          />
          <p lang={language} class="truncate text-lg font-bold">{endonym(language)}</p>
        </Field.Field>
      {/each}
    </Field.Group>
    <p class="text-sm text-muted-foreground">
      インストールしたばかりのフォントは、OBS を再起動すると配信に出ます。
    </p>
  </Card.Content>
</Card.Root>
