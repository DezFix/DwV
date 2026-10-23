export type DeleteMode = 'ask' | 'keep' | 'delete';

const STORAGE_KEY = 'dwv.delete-mode';

function readMode(): DeleteMode {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    return value === 'keep' || value === 'delete' ? value : 'ask';
  } catch {
    return 'ask';
  }
}

class DeletePrefsStore {
  mode = $state<DeleteMode>(readMode());

  setMode(mode: DeleteMode) {
    this.mode = mode;
    try {
      localStorage.setItem(STORAGE_KEY, mode);
    } catch {
      // Storage unavailable: keep the in-memory choice.
    }
  }
}

export const deletePrefs = new DeletePrefsStore();
