// Runtime configuration, loaded before the app.
//
// On the website the API is on the same origin, so apiBase stays empty.
// The mobile builds (Capacitor) load the app from the device, so their
// build step rewrites this file with the production API address, e.g.
//   window.QD_CONFIG = { apiBase: "https://qiyiqdoppi.uz" };
window.QD_CONFIG = { apiBase: "" };
