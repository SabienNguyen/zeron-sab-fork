-- Settings → Notifications' sounds: each chime can be a file of your own
-- (Choose…, its name under the row, Reset back to the default), and Test
-- plays it with its banner when banners are on, whatever its own switch
-- says. On the shell in ui/shell_fixture.luau, the host answered by
-- ui/settings_stub.luau (a `sound.play` with a file of its own is logged by
-- its path).

local PAGE = "#settings-page"
local NAV = "#settings-nav"

local function calls(app: any): { string }
	return (app:state("settings_stub_calls") :: any) or {}
end

local function count(app: any, entry: string): number
	local n = 0
	for _, c in calls(app) do
		if c == entry then
			n += 1
		end
	end
	return n
end

local function shows(app: any, scope: string, text: string): boolean
	return app:tree(scope):find(text, 1, true) ~= nil
end

local function open(): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("settings_stub.sign_in")
	app:command("shell.settings")
	app:frames(20)
	app:click(`{NAV} >> role:tab "Notifications"`)
	app:frames(30)
	return app
end

test("notification sounds: a chime's own file: chosen, named under its row, reset to the default", function()
	local app = open()
	expect(count(app, "sound.formats")).to_be(1)
	app:set_state("settings_stub_picked", "/home/ada/sounds/ding.ogg")
	app:click(`{PAGE} >> role:button "Choose a file for Input required sound"`)
	app:frames(10)
	expect((app:state("settings_sound_files") :: any).request).to_be("/home/ada/sounds/ding.ogg")
	expect(shows(app, PAGE, "ding.ogg")).to_be(true)
	-- (The others keep the default.)
	expect((app:state("settings_sound_files") :: any).done).to_be_nil()

	app:click(`{PAGE} >> role:button "Reset Input required sound to the default chime"`)
	app:frames(10)
	expect((app:state("settings_sound_files") :: any).request).to_be_nil()
	expect(shows(app, PAGE, "ding.ogg")).to_be(false)
end)

test("notification sounds: a file that isn't a sound played here isn't kept, and says so", function()
	local app = open()
	app:set_state("settings_stub_picked", "/home/ada/notes.txt")
	app:click(`{PAGE} >> role:button "Choose a file for Task completed sound"`)
	app:frames(10)
	expect((app:state("settings_sound_files") :: any).done).to_be_nil()
	expect(app:has('"That file can\'t be used as a sound"')).to_be(true)
end)

test("notification sounds: Test plays the chime, its own switch off, and posts its banner while banners are on", function()
	local app = open()
	app:set_state("settings_sound_done", false)
	app:frames(5)
	app:click(`{PAGE} >> role:button "Test Task completed sound"`)
	app:frames(5)
	expect(count(app, "sound.play:done")).to_be(1)
	expect(count(app, "notifications.test")).to_be(1)

	-- Its own file, there: that's what's played, nothing said.
	app:set_state("settings_sound_files", { done = "/home/ada/sounds/done.wav" })
	app:set_state("settings_stub_sounds_there", { ["/home/ada/sounds/done.wav"] = true })
	app:frames(5)
	app:click(`{PAGE} >> role:button "Test Task completed sound"`)
	app:frames(5)
	expect(count(app, "sound.play:/home/ada/sounds/done.wav")).to_be(1)
	expect(app:has('"done.wav can\'t be found"')).to_be(false)

	-- Banners off: the chime alone.
	app:click(`{PAGE} >> role:switch "Desktop notifications"`)
	app:frames(5)
	app:click(`{PAGE} >> role:button "Test Task completed sound"`)
	app:frames(5)
	expect(count(app, "sound.play:/home/ada/sounds/done.wav")).to_be(2)
	expect(count(app, "notifications.test")).to_be(2)
end)

test("notification sounds: Test with a file that's gone says the default plays in its place", function()
	local app = open()
	app:set_state("settings_sound_files", { attention = "/gone/alarm.wav" })
	app:frames(5)
	app:click(`{PAGE} >> role:button "Test Errors and disconnections sound"`)
	app:frames(10)
	expect(count(app, "sound.play:/gone/alarm.wav")).to_be(1)
	expect(app:has('"alarm.wav can\'t be found"')).to_be(true)
end)
