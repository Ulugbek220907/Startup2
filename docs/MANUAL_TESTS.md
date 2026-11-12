# Manual test checklist

Automated tests cover the server, rendering, pricing and the browser flow
with a simulated phone. These checks need real devices and cannot be
automated here. Record device, OS version and result for each run.

## Devices

- [ ] Low-end Android (2-3 GB RAM, e.g. Redmi 9A / Galaxy A03), Chrome
- [ ] Mid-range Android (e.g. Redmi Note 12 / Galaxy A34), Chrome
- [ ] The same Android phone, inside Telegram (Mini App)
- [ ] iPhone (iOS 16 or newer), Safari
- [ ] iPhone, inside Telegram
- [ ] Android app (APK from CI)
- [ ] iOS app (TestFlight)

Throttle to 4G where it matters: Chrome DevTools "Fast 4G", or a real SIM.

## 1. First load (target: usable in under 3 s on 4G)

- [ ] Catalog appears and is tappable in under 3 s on a cold cache
- [ ] Reload is near-instant (service worker)
- [ ] Airplane mode: the app shell still opens and shows "no connection"
      with a retry button
- [ ] Text is readable without zooming; nothing is cut off at 360 px width
- [ ] Language is right on first open (phone set to Uzbek, then Russian)

## 2. Editor

- [ ] Add text with oʻ and gʻ typed from the phone keyboard (try the
      keyboard's ʻ, the plain ' and the curly ’): all render in every font
- [ ] Cyrillic text renders in every font
- [ ] Drag, resize (yellow handle) and rotate (white handle) with a finger;
      the page does not scroll while dragging on the canvas
- [ ] Layer snaps to the centre line and the edges
- [ ] Upload a phone photo (10 MB+ is refused with a clear message)
- [ ] A small image stretched large shows the quality warning
- [ ] Draw with a finger; strokes follow without lag
- [ ] Undo/redo; close and reopen the app: the draft is still there
- [ ] Share link opens the same design on another phone

## 3. Live camera try-on (the least-tested feature: test it most)

For each device, with good light, phone propped up, standing 2 m away:

- [ ] Permission prompt appears once; "Allow" starts the camera
- [ ] First model load: time it on 4G. **Expected 8-15 s**; record the number
- [ ] Second open loads in under 2 s
- [ ] Garment sits on the shoulders and follows when you move and turn slightly
- [ ] Sleeves follow raised arms (T-shirt: to mid upper arm; hoodie: to the wrist)
- [ ] A hand held in front of the chest appears in front of the garment
- [ ] Frame rate feels smooth. If the browser has an FPS overlay, record it
      (**target: 20 fps or more on the mid-range phone**)
- [ ] Low-end phone: either it is usable, or the "device is slow" message
      appears with the photo option within about 10 s
- [ ] Stepping too close shows "step back"; turning sideways shows "face the camera"
- [ ] Switching the garment colour updates the preview
- [ ] Switch camera button works (rear camera is not mirrored)
- [ ] Snapshot opens the share sheet or saves an image
- [ ] Deny the permission: the message offers photo try-on and the plain view
- [ ] Leaving the screen turns the camera indicator off
- [ ] Phone stays cool enough to hold after 2 minutes
- [ ] iOS Safari and iOS Telegram specifically: video shows inline (not
      fullscreen), and the camera restarts after switching apps and back

## 4. Photo try-on

- [ ] Cannot pick a photo before ticking the consent box
- [ ] Dark photo, tiny photo, photo of two people, photo cut at the chest:
      each is refused with its own message
- [ ] A good photo produces a result (mock: garment pasted with a striped band)
- [ ] "Delete now" removes it; reopening the result link shows nothing
- [ ] With the real provider: go through `eval/README.md` first

## 5. Order and tracking

- [ ] Price changes when quantity, delivery region or pickup changes
- [ ] Wrong phone number is refused; order cannot be sent without the rights tick
- [ ] Order number is shown; "track" shows the timeline
- [ ] Admin changes the status: the customer's tracking page shows it after reload

## 6. Admin (desktop browser)

- [ ] New order appears; mockups show; print PNG and sheet download
- [ ] Open the print PNG in your RIP/print software: size is exactly the
      print area in cm at 300 DPI, background is transparent
- [ ] Print one real DTF transfer and press it: size and position match the mockup
- [ ] Scan the QR code on the printed sheet: it opens the order
- [ ] Price editor: change a price, confirm a new quote uses it and old orders do not change

## 7. Telegram

- [ ] `/start` in the bot, share phone: "phone linked"
- [ ] `/orders` lists orders; `/status CODE` shows one
- [ ] New order: the admin chat gets an alert with a working link
- [ ] Status change: the customer gets a message in their language
- [ ] Mini App opens from the bot button, in the right language, and
      "use my Telegram number" fills the phone field

## 8. Mobile apps

- [ ] Fresh install: native camera permission dialog with the right text
- [ ] Buttons give haptic feedback; share opens the native sheet
- [ ] Android back button goes back inside the app, exits from the catalog
- [ ] Open `qiyiqdoppi://d/<slug>` from a notes app: the design opens
- [ ] Airplane mode at launch: the app opens and shows the retry state
