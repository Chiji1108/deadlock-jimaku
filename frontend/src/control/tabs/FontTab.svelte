<script lang="ts">
  import { onMount } from "svelte";
  import PlayIcon from "@lucide/svelte/icons/play";
  import { Button } from "$lib/components/ui/button";
  import * as Card from "$lib/components/ui/card";
  import * as Field from "$lib/components/ui/field";
  import type { FontGroup } from "../../generated/FontGroup";
  import { SUBTITLE_SIZE } from "../../lib/overlay-size";
  import { applyOverlayFonts, FONT_SAMPLES, fontGroup } from "../../lib/overlay-fonts";
  import FontPicker from "../parts/FontPicker.svelte";
  import OverlayPreview from "../parts/OverlayPreview.svelte";
  import { panel } from "../panel.svelte";

  const s = $derived(panel.settings!);
  /** 字幕とランクに出る言語だけを並べる。ランクは名前がなければ英語で出るので、latin は常に出す */
  const groups = $derived([
    ...new Set(
      ["en", s.micLanguage, ...s.translateTo, s.rankLanguage ?? s.micLanguage].map(fontGroup),
    ),
  ]);
  const label = (group: FontGroup) =>
    group === "latin" ? "英語・キリル文字など" : panel.label(group);

  onMount(() => void panel.loadFonts());
  // 見本もオーバーレイと同じく lang.css の --font-* で描く
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
      {#each groups as group (group)}
        <Field.Field>
          <Field.Label for="font-{group}">{label(group)}</Field.Label>
          <FontPicker
            id="font-{group}"
            value={s.overlayFonts[group]}
            families={panel.fonts}
            onchange={(family) => panel.update({ overlayFonts: { [group]: family } })}
          />
          <p lang={group === "latin" ? "en" : group} class="truncate text-lg font-bold">
            {FONT_SAMPLES[group]}
          </p>
        </Field.Field>
      {/each}
    </Field.Group>
  </Card.Content>
</Card.Root>
