-- Sign out of the hub's page: the button in the bar ends the session of the tab at the hub, and the page goes to the
-- state that says it is not paired, on a copy of the fixture vault of the run (.testing.lua, app "hub": the debug
-- binary and the real engine, tools/sheet-hub.mjs). The unit tests and ui/e2e/app.spec.ts check the same through the
-- code; this sheet stays with the repository and runs again with every change, and leaves the shots for the judgement
-- of the bar. The steps find the button by its class, not its text: the page speaks the browser's language.
--   testing sheets step sign-out --headed
return {
  id = "rules-desktop.sign-out",
  title = "rules-desktop: Sign out in the bar ends the session and the page says it is not paired",
  task = "ALL/abnahme-abmelden-knopf-in-rules-testing-und-hub-desktop-im",
  engine = "browser",
  app = "hub",
  -- one size: the pairing link of the app is good for one pairing, and the app runs once for all sizes
  sizes = { { cols = 1000, rows = 700 } },
  steps = {
    { scheme = "light" },
    { open = "${link}", note = "the pairing link the hub printed" },
    { wait = "document.querySelector('header button.sign-out') !== null && document.querySelector('nav') !== null", timeout = 20000, note = "paired: the views and the button are there" },
    { expect = "/^[0-9a-f]{64}$/.test(sessionStorage.getItem('rules-desktop.session') ?? '')", note = "the tab holds a session" },
    { shot = "bar-light" },
    { scheme = "dark" },
    { shot = "bar-dark" },
    { hover = "header button.sign-out" },
    { expect = "document.querySelector('header button.sign-out').matches(':hover')" },
    { shot = "hover-dark" },
    { click = "header button.sign-out", note = "the person signs out" },
    { wait = "document.querySelector('header button.sign-out') === null && document.querySelector('section.state') !== null", timeout = 10000, note = "the page says it has no session" },
    { expect = "sessionStorage.getItem('rules-desktop.session') === null", note = "the token is gone from the tab" },
    { expect = "document.querySelector('nav') === null", note = "the views are gone: nothing of the rule sets is shown" },
    { shot = "signed-out" },
  },
  judge = {
    "Sign out sits at the right end of the bar, is readable in the light and the dark scheme, and its border shows against the bar",
    "After the click the page says it is not paired and shows nothing of the rule sets",
  },
}
