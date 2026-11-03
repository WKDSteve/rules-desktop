// The tokens and base rules of appkit-ui first: the look every app of appkit shares.
import 'appkit-ui/tokens.css';
import 'appkit-ui/base.css';
import { askForSession, shareSession, type ChannelLike } from 'appkit-ui/handoff';
import { placeOf } from 'appkit-ui/nav';
import { takePairingCode } from 'appkit-ui/pair';
import { installTooltips } from 'appkit-ui/tooltip';
import { mount } from 'svelte';
import App from './App.svelte';
import { Hub } from './lib/api';
import { App as AppState, navRoute, routeNav, TOKEN_KEY, type Env } from './lib/app.svelte';

const target = document.getElementById('app');
if (target === null) throw new Error('#app is missing');

// The code leaves the address bar before anything else happens; the place of the page in the fragment stays.
const code = takePairingCode(window.location, window.history, undefined, { keepRoute: true });
// The view (and the rule open in it) the address names: a link opened in a new tab, a reload.
const place = placeOf({ location: window.location, toRoute: navRoute, fromRoute: routeNav });

// The tabs of the page hand their session to a new one (appkit-ui/handoff). A browser without the channel pairs as before.
let channel: ChannelLike | null = null;
try {
  channel = new BroadcastChannel(`${TOKEN_KEY}.handoff`);
} catch {
  channel = null;
}

const fetchFn: typeof fetch = (input, init) => window.fetch(input, init);
const env: Env = {
  hub: new Hub(fetchFn),
  session: window.sessionStorage,
  prefs: window.localStorage,
  preferredLanguages: navigator.languages,
  now: () => Date.now(),
  fetchFn,
  sleep: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  ...(channel
    ? {
        askSession: () =>
          askForSession({
            channel: channel as ChannelLike,
            random: () => crypto.getRandomValues(new Uint8Array(16)),
            setTimeout: (run, ms) => window.setTimeout(run, ms),
            clearTimeout: (handle) => window.clearTimeout(handle as number),
          }),
      }
    : {}),
};

const state = new AppState(env);
if (place !== null) state.openPlace(place);
if (channel) {
  shareSession({
    channel,
    token: () => {
      try {
        return window.sessionStorage.getItem(TOKEN_KEY);
      } catch {
        return null;
      }
    },
    live: () => state.stream.open,
  });
}
installTooltips(document, window, { regionLabel: () => 'hints' });
mount(App, { target, props: { app: state, code } });

// A link of `rules-hub open` pasted into a tab that is already open changes only the fragment: no reload. The
// page starts again with the new code; what ran before is stopped first.
window.addEventListener('hashchange', () => {
  // A place typed into the address is not the pairing's: it stays (the history goes to it).
  const later = takePairingCode(window.location, window.history, undefined, { keepRoute: true });
  if (later) {
    state.stop();
    void state.start(later);
  }
});
