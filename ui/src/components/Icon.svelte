<script lang="ts">
  import { icons, type IconName } from '../assets/icons';

  interface Props {
    name: IconName;
    size?: number;
  }

  let { name, size = 16 }: Props = $props();
</script>

<span class="icon" style="width: {size}px; height: {size}px">{@html icons[name]}</span>

<style>
  .icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .icon :global(svg) {
    width: 100%;
    height: 100%;
    display: block;
  }
</style>
