<script lang="ts">
let {
  top = "",
  middle = "",
  bottomLeft = "",
  bottomRight = ""
}: {
  top?: string;
  middle?: string;
  bottomLeft?: string;
  bottomRight?: string;
} = $props();
</script>

<header class="queue-header">
  <div class="header-content">
    <div class="header-top-row">
      <span class="v-truncate header-title">{top}</span>
    </div>
    <div class="header-middle-row">
      <span class="v-truncate header-artist">{middle}</span>
    </div>
    {#if bottomLeft || bottomRight}
      <div class="header-bottom-row">
        {#if bottomLeft}
          <span class="v-mono header-badge">{bottomLeft}</span>
        {:else}
          <div></div>
        {/if}
        {#if bottomRight}
          <span class="v-mono header-badge">{bottomRight}</span>
        {/if}
      </div>
    {/if}
  </div>
</header>

<style>
.queue-header {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  box-sizing: border-box;
  width: 100%;
}

.header-content {
  flex: 1;
  display: flex;
  flex-direction: column;
  justify-content: center;
  min-width: 0;
  gap: 6px;
}

.header-top-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  width: 100%;
}

.header-middle-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  width: 100%;
}

.header-bottom-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  width: 100%;
  margin: 6px 0 10px;
}

.header-title {
  color: var(--text-main);
  font-size: 19px;
}

.header-artist {
  color: var(--text-muted);
  font-size: 17px;
}

.header-badge {
  display: flex;
  align-items: center;
  padding: 0 12px;
  font-size: 12px;
  font-weight: 400;
  border: 1px solid var(--border-muted);
  border-radius: 8px;
  height: 24px;
  box-sizing: border-box;
  color: var(--text-muted);
}
</style>
