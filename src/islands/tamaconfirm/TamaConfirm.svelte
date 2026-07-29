<script lang="ts">
  import { tamaConfirmCtrl } from "./tamaconfirm.svelte.ts";
  import * as bridge from "../../legacy/bridge";

  // Tama's face matches the tone: alarmed for a warning/danger, curious for info.
  const face = $derived(
    tamaConfirmCtrl.kind === "info" ? bridge.TAMA_IMG.curious : bridge.TAMA_IMG.alarm,
  );

  function onKeydown(e: KeyboardEvent) {
    if (!tamaConfirmCtrl.open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      tamaConfirmCtrl.cancel();
    } else if (e.key === "Enter") {
      e.preventDefault();
      tamaConfirmCtrl.confirm();
    }
  }
</script>

<svelte:window on:keydown={onKeydown} />

<div class="scrim" class:on={tamaConfirmCtrl.open}>
  <div class="modal tamaconfirm">
    <div class="modal-head">
      <div class="modal-tama"><img class="tama-pic" src={face} alt="Tama" /></div>
      <div>
        <h3>{tamaConfirmCtrl.title}</h3>
      </div>
    </div>
    <div class="modal-body">
      <p class="tc-msg">{tamaConfirmCtrl.message}</p>
    </div>
    <div class="modal-foot">
      <button class="btn ghost" onclick={() => tamaConfirmCtrl.cancel()}>{tamaConfirmCtrl.cancelLabel}</button>
      <button
        class="btn"
        class:danger={tamaConfirmCtrl.kind === "danger"}
        onclick={() => tamaConfirmCtrl.confirm()}>{tamaConfirmCtrl.confirmLabel}</button
      >
    </div>
  </div>
</div>
