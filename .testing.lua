-- .testing.lua -- what the test sheets of this repository run against (testing.nvim, docs/SHEETS.md there).
-- The sheets in TESTS/sheets only name an app; how it starts is here, under testing.nvim's trust gate.

return {
  sheets = {
    -- the Chrome of the machine, as the e2e tests; playwright-core comes at the version of ui/pnpm-lock.yaml
    browser = { playwright_dir = "ui", channel = "chrome" },
    apps = {
      -- The hub of the run on a copy of the fixture vault (tools/sheet-hub.mjs): the debug binary and the real engine
      -- (RULES_ENGINE_DIR, LIB_NVIM_DIR), as ui/e2e/hub.ts starts them, in a folder of the run. The page is built first
      -- (the binary embeds ui/dist). The starter prints the pairing link once the engine answers and ends the hub when
      -- the sheet is done.
      ["hub"] = {
        build = {
          { "pnpm", "--dir", "ui", "build" },
          { "cargo", "build", "-p", "rules-hub", "--locked" },
        },
        run = { "node", "tools/sheet-hub.mjs", "${temp}", "47398" },
        ready = { stdout = "^http://127%.0%.0%.1:%d+/#pair=%x+$", as = "link" },
        env = { NVIM_MACHINE_ROLE = "workstation" },
      },
      -- The release build of rules-hub (a window program) started the way a link starts it (testing.nvim `start =
      -- "link"`: no standard handles, no console of a parent), against a config.json it cannot use in a folder of the
      -- run. The hub in the background ends at once on the config; the start sees that within about 2 s (appkit
      -- v0.62.0) and the program says so in a message box that names the cause from hub.log, its only window. Nothing
      -- outlives the sheet (the hub in the background ends at once on the config), and the person's settings and hub
      -- stay untouched (own folder, own port, no browser).
      ["link-start-error"] = {
        build = { "cargo", "build", "--release", "-p", "rules-hub", "--locked" },
        prepare = {
          "python",
          "-c",
          "import pathlib, sys; p = pathlib.Path(sys.argv[1]); p.parent.mkdir(parents=True, exist_ok=True);"
            .. " p.write_text(sys.argv[2])",
          "${temp}/cfg/config.json",
          '{ "port": ',
        },
        run = { "target/release/rules-hub.exe" },
        start = "link",
        window = "rules-hub",
        window_ms = 15000,
        env = {
          RULES_DESKTOP_CONFIG_DIR = "${temp}/cfg",
          RULES_DESKTOP_PORT = "47396",
          RULES_DESKTOP_NO_LAUNCH = "1",
          NVIM_MACHINE_ROLE = "workstation",
        },
      },
    },
  },
}
