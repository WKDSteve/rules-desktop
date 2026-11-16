-- A start from the link that fails: the release build of rules-hub is a window program, so nobody reads its stderr
-- there and the error must come as a message box (appkit console::report). The app `link-start-error` of .testing.lua
-- starts it as a link does, against a config.json it cannot use; the message box is its only window. Without the start
-- as by a link (stderr read by the run) the program shows no window and the sheet is red. What stays for a person is
-- in the manual sheet window-program-live (the real Start menu link and login).
--   testing sheets run --sheet link-start-error
return {
  id = "rules-desktop.link-start-error",
  title = "rules-hub: a start from a link that fails says so in a message box",
  task = "rules-desktop/handlauf-hub-als-fenster-programm-im-echten-profil",
  engine = "desktop",
  app = "link-start-error",
  sizes = { { cols = 640, rows = 240 } },
  steps = {
    { wait = "OK", timeout = 5000, note = "the message box of rules-hub is up, with its button" },
    { expect = { contains = "EOF while parsing" }, note = "it says why: the broken config.json" },
    { expect = { contains = "hub.log" }, note = "and where to read more" },
    { shot = "message" },
    { click = "OK", note = "the person closes it" },
  },
  judge = { "The message is readable: the cause and hub.log are checked by the steps, the box must not cut them off" },
}
