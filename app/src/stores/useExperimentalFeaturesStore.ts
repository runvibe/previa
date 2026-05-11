import { useSyncExternalStore } from "react";

export const EXPERIMENTAL_FEATURES_STORAGE_KEY = "previa-experimental-features-enabled";

const EXPERIMENTAL_FEATURES_CHANGED_EVENT = "previa:experimental-features-changed";

export function readExperimentalFeaturesEnabled() {
  return localStorage.getItem(EXPERIMENTAL_FEATURES_STORAGE_KEY) !== "false";
}

function subscribeExperimentalFeatures(listener: () => void) {
  const handleStorage = (event: StorageEvent) => {
    if (event.key === EXPERIMENTAL_FEATURES_STORAGE_KEY) listener();
  };

  window.addEventListener("storage", handleStorage);
  window.addEventListener(EXPERIMENTAL_FEATURES_CHANGED_EVENT, listener);

  return () => {
    window.removeEventListener("storage", handleStorage);
    window.removeEventListener(EXPERIMENTAL_FEATURES_CHANGED_EVENT, listener);
  };
}

export function setExperimentalFeaturesEnabled(enabled: boolean) {
  localStorage.setItem(EXPERIMENTAL_FEATURES_STORAGE_KEY, String(enabled));
  window.dispatchEvent(new Event(EXPERIMENTAL_FEATURES_CHANGED_EVENT));
}

export function useExperimentalFeaturesEnabled() {
  return useSyncExternalStore(
    subscribeExperimentalFeatures,
    readExperimentalFeaturesEnabled,
    () => true,
  );
}
