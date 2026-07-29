<script lang="ts">
  import { mainlinePickerCtrl } from "./mainlinepicker.svelte.ts";
  import * as bridge from "../../legacy/bridge";

  // Parent 1 is the branch the merge was made ON (merged into); parent 2 is the
  // branch merged in. Spell that out so the choice isn't just "1 or 2".
  function role(n: number): string {
    if (n === 1) return "the branch merged into (mainline — usually this one)";
    if (n === 2) return "the branch merged in";
    return `parent ${n}`;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && mainlinePickerCtrl.open) mainlinePickerCtrl.cancel();
  }
</script>

<svelte:window on:keydown={onKeydown} />

<div class="scrim" class:on={mainlinePickerCtrl.open}>
  <div class="modal mainline">
    <div class="modal-head">
      <div class="modal-tama"><img class="tama-pic" src={bridge.TAMA_IMG.curious} alt="Tama, curious" /></div>
      <div>
        <h3>Cherry-pick a merge commit</h3>
        <p>
          <span class="mono">{mainlinePickerCtrl.sha.slice(0, 8)}</span> is a merge, so git needs to know which parent is the
          mainline — the changes it brings in are measured against that parent.
        </p>
      </div>
    </div>
    <div class="modal-body">
      <div class="ml-list">
        {#each mainlinePickerCtrl.parents as p (p.number)}
          <button class="ml-parent" onclick={() => mainlinePickerCtrl.pick(p.number)}>
            <span class="ml-num">-m {p.number}</span>
            <span class="ml-body">
              <span class="ml-sha mono">{p.shortSha}</span>
              <span class="ml-summary">{p.summary}</span>
              <span class="ml-role mut">{role(p.number)}</span>
            </span>
          </button>
        {/each}
      </div>
    </div>
    <div class="modal-foot">
      <button class="btn ghost" onclick={() => mainlinePickerCtrl.cancel()}>Cancel</button>
    </div>
  </div>
</div>
