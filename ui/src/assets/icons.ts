import videoIcon from '../assets/icons/video.svg?raw';
import audioIcon from '../assets/icons/audio.svg?raw';
import documentIcon from '../assets/icons/document.svg?raw';
import archiveIcon from '../assets/icons/archive.svg?raw';
import programIcon from '../assets/icons/program.svg?raw';
import otherIcon from '../assets/icons/other.svg?raw';
import downloadIcon from '../assets/icons/download.svg?raw';
import folderIcon from '../assets/icons/folder.svg?raw';
import menuIcon from '../assets/icons/menu.svg?raw';
import settingsIcon from '../assets/icons/settings.svg?raw';
import playIcon from '../assets/icons/play.svg?raw';
import pauseIcon from '../assets/icons/pause.svg?raw';
import plusIcon from '../assets/icons/plus.svg?raw';
import searchIcon from '../assets/icons/search.svg?raw';
import closeIcon from '../assets/icons/close.svg?raw';
import externalIcon from '../assets/icons/external.svg?raw';
import homeIcon from '../assets/icons/home.svg?raw';
import globeIcon from '../assets/icons/globe.svg?raw';
import paletteIcon from '../assets/icons/palette.svg?raw';
import slidersIcon from '../assets/icons/sliders.svg?raw';
import checkIcon from '../assets/icons/check.svg?raw';
import clockIcon from '../assets/icons/clock.svg?raw';
import minusIcon from '../assets/icons/minus.svg?raw';
import chevronUpIcon from '../assets/icons/chevronUp.svg?raw';
import chevronDownIcon from '../assets/icons/chevronDown.svg?raw';
import snailIcon from '../assets/icons/snail.svg?raw';
import heartIcon from '../assets/icons/heart.svg?raw';

export const icons = {
  video: videoIcon,
  audio: audioIcon,
  document: documentIcon,
  archive: archiveIcon,
  program: programIcon,
  other: otherIcon,
  download: downloadIcon,
  folder: folderIcon,
  menu: menuIcon,
  settings: settingsIcon,
  play: playIcon,
  pause: pauseIcon,
  plus: plusIcon,
  search: searchIcon,
  close: closeIcon,
  external: externalIcon,
  home: homeIcon,
  globe: globeIcon,
  palette: paletteIcon,
  sliders: slidersIcon,
  check: checkIcon,
  clock: clockIcon,
  minus: minusIcon,
  chevronUp: chevronUpIcon,
  chevronDown: chevronDownIcon,
  snail: snailIcon,
  heart: heartIcon,
} as const;

export type IconName = keyof typeof icons;
