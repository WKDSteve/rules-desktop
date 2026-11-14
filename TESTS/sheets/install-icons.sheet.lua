-- The hand check after the icons of the family changed: install rules again and look at it where Windows shows it.
-- A Claude session may not install (it sees a redirected AppData and HKCU), so a person does it; the sheet asks.
-- Whether the installed program carries the current icon needs no eye: the second question runs the check of the
-- icon generator (tasks-vault TOOLS/scripts/app-icons, icons.mjs exe), which compares the images byte for byte.
--   testing sheets step install-icons
return {
	id = "rules-desktop.install-icons",
	title = "rules-desktop: installed again, the program, its links and its window show the current icon",
	task = "ALL/handlauf-hub-rules-und-docmap-neu-installieren-damit-program",
	engine = "manual",
	steps = {
		{ ask = "In a normal terminal (not a Claude session), scripts\\build-install.ps1 built and installed rules?" },
		{
			ask = "node $REPOS_DIR/tasks-vault/TOOLS/scripts/app-icons/icons.mjs exe"
				.. " $env:LOCALAPPDATA\\rules-desktop\\bin\\rules-hub.exe rules-desktop says: carries the current icon?",
		},
		{ ask = "The Start menu link Rules shows the rules icon (the bar with rl)?" },
		{
			ask = "The desktop link Rules shows the same icon, taken from the program"
				.. " (Properties, Change Icon names rules-hub.exe, no file below the repos)?",
		},
		{ ask = "The window of rules and its taskbar button show the new icon?" },
		{
			ask = "Explorer shows the new icon on the program and the links"
				.. " (an old picture that sticks: ie4uinit -show, then look again)?",
		},
	},
}
