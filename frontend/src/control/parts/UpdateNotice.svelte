<script lang="ts">
  // 新しい版があることを、操作パネルに出し続ける（DESIGN.md 13.5）。起動したときのダイアログを「あとで」で閉じた人や、
  // 常駐させたまま起動し直さない人が、古い版のままにならないようにする。配信の途中でも見えるが、勝手には更新しない
  import DownloadIcon from "@lucide/svelte/icons/download";
  import { toast } from "svelte-sonner";
  import * as Alert from "$lib/components/ui/alert";
  import { Button } from "$lib/components/ui/button";
  import { api } from "../../lib/api";
  import { panel } from "../panel.svelte";

  /** 頼んでいる間。入れ替えるのはデスクトップアプリで、終わると再起動する（できなかったときは、アプリがダイアログで
   * 知らせる。もう一度押せるように、ボタンは戻す） */
  let asking = $state(false);

  async function update() {
    asking = true;
    try {
      await api.updateApp();
      toast.success("更新を始めました。ダウンロードが終わると、アプリが再起動します");
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      asking = false;
    }
  }
</script>

{#if panel.view?.update}
  <Alert.Root>
    <DownloadIcon />
    <Alert.Title>新しい版（{panel.view.update}）があります</Alert.Title>
    <Alert.Description class="flex flex-col gap-2">
      <span>
        今の版は {panel.view.version} です。更新するとアプリが再起動します{panel.running
          ? "（字幕は一度止まります）"
          : ""}
      </span>
      <span>
        <Button size="sm" disabled={asking} onclick={update}><DownloadIcon />更新する</Button>
      </span>
    </Alert.Description>
  </Alert.Root>
{/if}
