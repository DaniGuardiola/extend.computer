// The preview has no webview; CSS zoom on the page stands in for page zoom.
export const getCurrentWebview = () => ({
  setZoom: async (scale: number) => {
    document.documentElement.style.zoom = String(scale);
  },
});
