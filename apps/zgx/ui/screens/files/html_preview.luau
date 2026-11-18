-- An HTML file's Preview in the file viewer (ds/file_viewer's `html_view`):
-- the page as a browser draws it, from the editor's text, so what is typed
-- and not saved yet shows. The shipping app laid the page out itself
-- (crates/ui/src/files/html_preview.rs, no scripts, nothing fetched); here
-- it is a web page of its own (zgx's, as a Browser tab's), its scripts run.
--
-- On this machine, the page is given the file's own address as its base,
-- so what it names beside it (its pictures, styles, scripts) loads, and its
-- anchors stay in it. On another device there is no such address: the page
-- shows alone, its relative parts missing.
--
-- A web link followed in it opens in a Browser tab (`open_url`), the
-- preview staying on the file; so does a page it asks a window for.
--
--   HtmlPreview { view, path, absolute, text, open_url? }
--
-- `view`: the files view's id, so each tab's preview is its own page.
-- `text` and `absolute` (the file on this machine, nil while it isn't
-- known to be) are read as they change: the page follows the text a beat
-- after it settles, the address at once.

local api = require("@app/screens/panel/browser_api")

-- How long the text stays put before the page shows it again (one load per
-- pause, not per change).
local SETTLE = 250

export type Props = {
	view: string,
	path: string,
	absolute: () -> string?,
	text: () -> string,
	open_url: ((url: string) -> ())?,
	[any]: any,
}

-- A `file:` address for a path on this machine: each part's bytes past the
-- unreserved escaped, as a browser writes them.
local function file_url(path: string): string
	local escaped = string.gsub(path, "[^%w%-%._~/]", function(c: string): string
		return string.format("%%%02X", string.byte(c))
	end)
	return `file://{escaped}`
end

local M = {}
M.file_url = file_url

M.HtmlPreview = ui.component(function(props: Props): Node
	-- (A page of its own, kept apart from the browser's sign-ins.)
	local web = api.web(`files-html-{props.view}`, { profile = "ephemeral" })
	local shown: string? = nil
	local base: string? = nil
	local function show(text: string)
		shown = text
		web:html(text, base)
	end
	-- The file's text, at once, then a beat after each change settles.
	local pending: (() -> ())? = nil
	ui.effect(function()
		local text = props.text()
		local at = props.absolute()
		local now = if at then file_url(at) else nil
		if pending then
			pending()
			pending = nil
		end
		if shown == nil or now ~= base then
			base = now
			show(text)
		elseif text ~= shown then
			pending = ui.after(SETTLE, function()
				pending = nil
				show(text)
			end)
		end
	end)
	ui.on_cleanup(function()
		if pending then
			pending()
		end
		-- (Shown again, it's made again: no page goes on unseen.)
		pcall(function()
			web:close()
		end)
	end)
	local function away(url: string)
		if props.open_url and api.allowed(url) then
			props.open_url(url)
		end
	end
	return api.view {
		id = `files-html-{props.view}`,
		label = ui.t("files.html_preview", { name = string.match(props.path, "[^/]+$") or props.path }),
		web = web,
		w_full = true,
		h_full = true,
		on_navigate = function(e: any)
			-- A web link followed: to a Browser tab, and the file back here.
			-- (Only the web's: the file's own address, its anchors and what
			-- it names beside it are the preview's, so its load never comes
			-- back here.)
			if type(e) == "table" and e.kind == "started" and type(e.url) == "string" and api.allowed(e.url) then
				away(e.url)
				ui.after(0, function()
					if shown then
						show(shown)
					end
				end)
			end
		end,
		on_new_window = away,
	}
end)

return M
