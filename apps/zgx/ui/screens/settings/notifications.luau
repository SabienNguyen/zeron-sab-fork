-- Settings → Notifications: the shipping app's (crates/ui
-- settings/notifications.rs). Desktop banners are the engine's to post (it
-- keeps posting them with the window closed), so their switches are its
-- file's (`notifications.get`/`.set`, src/settings.rs); the session chimes
-- are this app's (prefs.luau, played by chimes.luau). Each sub-setting
-- stays put while its parent is off, dimmed and out of reach. Each chime
-- can be a file of the person's own, and heard with its banner (Test).

local host = require("@app/screens/settings/host")
local ds = require("@ds")
local toast = require("@ds/toast")
local prefs = require("@app/screens/settings/prefs")
local search = require("@app/screens/settings/search")
local visits = require("@app/screens/settings/visits")

local M = {}

M.ID = "notifications"
M.TITLE = "Notifications"

search.index(M.ID, ui.t("settings.notifications.title"), ui.t("settings.notifications.terms"), {
	{ key = "desktop", title = ui.t("settings.notifications.desktop.label"), section = ui.t("settings.notifications.desktop_section"), terms = ui.t("settings.notifications.desktop.terms") },
	{ key = "agent_updates", title = ui.t("settings.notifications.agent_updates.label"), section = ui.t("settings.notifications.desktop_section"), terms = ui.t("settings.notifications.agent_updates.terms") },
	{ key = "background_only", title = ui.t("settings.notifications.background_only.label"), section = ui.t("settings.notifications.desktop_section"), terms = ui.t("settings.notifications.background_only.terms") },
	{ key = "sounds", title = ui.t("settings.notifications.sounds.label"), section = ui.t("settings.notifications.sounds_section"), terms = ui.t("settings.notifications.sounds.terms") },
	{ key = "sound_done", title = ui.t("settings.notifications.sound_done.label"), section = ui.t("settings.notifications.sounds_section"), terms = ui.t("settings.notifications.sound_done.terms") },
	{ key = "sound_request", title = ui.t("settings.notifications.sound_request.label"), section = ui.t("settings.notifications.sounds_section"), terms = ui.t("settings.notifications.sound_request.terms") },
	{ key = "sound_attention", title = ui.t("settings.notifications.sound_attention.label"), section = ui.t("settings.notifications.sounds_section"), terms = ui.t("settings.notifications.sound_attention.terms") },
})

-- The banners' switches, as the engine's file has them (on, unread).
local desktop = ui.state("settings_banners", true)
local background_only = ui.state("settings_banners_background", true)
local agent_updates = ui.state("settings_banners_updates", true)
-- Whether they couldn't be read (the switches then hold still, as they'd
-- say what isn't so).
local unread = ui.state("settings_banners_unread", false)

local function load()
	host.call("notifications.get"):after(function(v: any)
		ui.batch(function()
			unread:set(false)
			desktop:set(v.desktop)
			background_only:set(v.background_only)
			agent_updates:set(v.agent_updates)
		end)
	end, function()
		unread:set(true)
	end)
end

-- A flip, written to the engine's file; back as it was if it can't be.
local function banner_switch(label: Label, value: State<boolean>, field: string): Node
	return ds.Switch {
		value = value,
		label = label,
		on_change = function(on: boolean)
			-- (Unread, the file isn't written over with a guess.)
			if unread:peek() then
				value:set(not on)
				return
			end
			host.call("notifications.set", { [field] = on }):after(function() end, function()
				value:set(not on)
				toast.show { title = ui.t("settings.engine.couldnt_save"):get(), body = ui.t("settings.engine.offline"):get(), tone = "danger" }
			end)
		end,
	}
end

-- A row that hangs off another: dimmed and out of reach while `parent` is
-- off.
local function under(parent: State<boolean>, key: string, props: any): Node
	return search.anchor(
		key,
		ui.div {
			w_full = true,
			opacity = function()
				return if parent:get() then 1 else "$zeron.disabled"
			end,
			disabled = function()
				return not parent:get()
			end,
			ds.Settings.Row(props),
		}
	)
end

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

-- The extensions a chime's own file may have on this computer (the host's
-- `sound.formats`), once read: the picker's filter. Empty, unread: any file.
local formats = ui.state("settings_sound_formats", {} :: { string })

local function load_formats()
	host.call("sound.formats"):after(function(v: any)
		formats:set(if type(v) == "table" then v else {})
	end, function() end)
end

-- A path's last part, its file's name.
local function file_name(path: string): string
	return string.match(path, "([^/\\]+)[/\\]?$") or path
end

local function t(key: string, args: any?): string
	return ui.t(`settings.notifications.{key}`, args):get()
end

-- A chime's own file, chosen: kept unless the host says it isn't a sound
-- played here (unasked, kept: the host plays the default in its place).
local function choose(kind: string)
	local f = formats:peek()
	host.open_files({
		title = t("sound_file.picker_title"),
		filters = if #f > 0 then { { t("sound_file.filter"), f } } else nil,
	}):after(function(files: { File })
		local file = files and files[1]
		if not file then
			return
		end
		host.call("sound.check", { path = file.path }):after(function(word: any)
			if word == "unsupported" then
				local list = {}
				for _, ext in formats:peek() do
					table.insert(list, string.upper(ext))
				end
				toast.show {
					title = t("sound_file.unsupported_title"),
					body = if #list > 0 then t("sound_file.unsupported_body", { formats = table.concat(list, ", ") }) else nil,
					tone = "warning",
				}
			else
				prefs.set_sound_file(kind, file.path)
			end
		end, function()
			prefs.set_sound_file(kind, file.path)
		end)
	end)
end

-- A chime heard as a session would ring it, with its banner when banners
-- are on. Asked for, so its own switch and "only when in the background"
-- don't hold it. A file of its own that's gone is said (the default plays).
local function test(kind: string)
	host.invoke("sound.play", prefs.sound_ask(kind))
	if desktop:peek() then
		host.invoke("notifications.test", { title = t("test_banner.title"), body = t(`test_banner.{kind}`) })
	end
	local path = prefs.sound_file(kind)
	if path then
		host.call("sound.check", { path = path }):after(function(word: any)
			if word == "missing" then
				toast.show { title = t("sound_file.missing_title", { file = file_name(path) }), body = t("sound_file.missing_body"), tone = "warning" }
			end
		end, function() end)
	end
end

-- A chime's row: its own file's name under it (Reset at its end), Test and
-- Choose… beside its switch.
local function sound_row(kind: string): Node
	local key = `sound_{kind}`
	local function named(what: string): () -> string
		return function(): string
			return t(`sound_file.{what}_named`, { name = t(`{key}.name`) })
		end
	end
	return under(prefs.sounds, key, {
		label = ui.t(`settings.notifications.{key}.label`),
		detail = function(): string?
			local path = prefs.sound_file(kind)
			return if path then file_name(path) else nil
		end,
		detail_action = ds.Button {
			ui.t("settings.notifications.sound_file.reset"),
			tone = "ghost",
			size = "xs",
			label = named("reset"),
			on_click = function()
				prefs.set_sound_file(kind, nil)
			end,
		},
		control = ui.row {
			gap = "$space.2",
			items = "center",
			ds.Button {
				ui.t("settings.notifications.sound_file.test"),
				tone = "ghost",
				size = "sm",
				label = named("test"),
				on_click = function()
					test(kind)
				end,
			},
			ds.Button {
				ui.t("settings.notifications.sound_file.choose"),
				tone = "secondary",
				size = "sm",
				label = named("choose"),
				on_click = function()
					choose(kind)
				end,
			},
			ds.Switch { value = prefs.sound(kind), label = ui.t(`settings.notifications.{key}.name`) },
		},
	})
end

function M.view(): Node
	visits.once("notifications", load)
	visits.once("notification_sounds", load_formats)
	return ds.Settings.Page {
		title = ui.t("settings.notifications.title"),
		ds.Settings.Section {
			title = ui.t("settings.notifications.desktop_section"),
			ui.show(function(): boolean
				return unread:get()
			end, ds.Callout {
				tone = "warning",
				title = ui.t("settings.notifications.unread.title"),
				body = ui.t("settings.notifications.unread.body"),
				action = {
					label = ui.t("common.retry") :: any,
					on_click = function()
						visits.again("notifications", load)
					end,
				},
			}),
			ds.Settings.Group {
				row("desktop", {
					label = ui.t("settings.notifications.desktop.label"),
					description = ui.t("settings.notifications.desktop.description"),
					control = banner_switch(ui.t("settings.notifications.desktop.label"), desktop, "desktop"),
				}),
				under(desktop, "agent_updates", {
					label = ui.t("settings.notifications.agent_updates.label"),
					description = ui.t("settings.notifications.agent_updates.description"),
					control = banner_switch(ui.t("settings.notifications.agent_updates.label"), agent_updates, "agent_updates"),
				}),
				under(desktop, "background_only", {
					label = ui.t("settings.notifications.background_only.label"),
					description = ui.t("settings.notifications.background_only.description"),
					control = banner_switch(ui.t("settings.notifications.background_only.label"), background_only, "background_only"),
				}),
			},
		},
		ds.Settings.Section {
			title = ui.t("settings.notifications.sounds_section"),
			ds.Settings.Group {
				row("sounds", {
					label = ui.t("settings.notifications.sounds.label"),
					control = ds.Switch { value = prefs.sounds, label = ui.t("settings.notifications.sounds.label") },
				}),
				sound_row("done"),
				sound_row("request"),
				sound_row("attention"),
			},
		},
	}
end

return M
