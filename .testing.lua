-- .testing.lua -- what the test sheets of this repository run against (testing.nvim, docs/SHEETS.md there).
-- The sheets in TESTS/sheets only name an app; how it starts is here, under testing.nvim's trust gate.

return {
  sheets = {
    apps = {
      -- The release build of rules-hub (a window program) started the way a link starts it (testing.nvim
      -- `start = "link"`: no standard handles, no console of a parent), against a config.json it cannot use in a folder
      -- of the run. The start in the background gives up after 20 s and the program says so in a message box, its only
      -- window. Nothing outlives the sheet (the hub in the background ends at once on the config), and the person's
      -- settings and hub stay untouched (own folder, own port, no browser).
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
        window_ms = 90000,
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
