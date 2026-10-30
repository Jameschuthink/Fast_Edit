// fx bottomfade: a dark gradient shades the picture top-down from y to y+height (captions and words stay bright). keys: y=660 height=130 alpha=0.65 color=#000000
// made by note n37 (a test job)
FX.def("bottomfade", { keys: { y: 660, height: 130, alpha: 0.65, color: "#000000" } }, {
  video(g, f, k) { const gr = g.createLinearGradient(0, k.y, 0, k.y + k.height); gr.addColorStop(0, k.alpha === 1 ? k.color : k.color + Math.round(k.alpha * 255).toString(16).padStart(2, "0")); gr.addColorStop(1, k.color + "00"); g.fillStyle = gr; g.globalAlpha = 1; g.fillRect(0, k.y, f.W, f.H - k.y); },
});
