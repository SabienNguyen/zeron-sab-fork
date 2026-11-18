-- An HTML file in a file tab: its page is its Preview (a web page made
-- from the editor's text, its base the file's own address), its source a
-- switch away, and what's typed there shows in the page. Its globe opens a
-- Browser tab of the file (the same tab again for the same file), made
-- from the text with no file address given; the tab says the file's path
-- and name, an anchor in it keeps them, and Reload makes the page again
-- from the text. On the panel in ui/panel_fixture.luau, its pages stood in
-- for, over the made-up disk of ui/files_stub.luau.

local PAGE = '<h1>Page</h1>\n<a href="#end">End</a>\n<p id="end">End</p>\n'

local function eventually(app: any, ok: () -> boolean, ms: number?): boolean
	local tries = math.ceil((ms or 15000) / 50)
	for _ = 1, tries do
		if ok() then
			return true
		end
		app:wait('"never shown, only waited past"', { timeout_ms = 50 })
	end
	return ok()
end

local function web_calls(app: any): string
	return app:state("test_web_calls") or ""
end

local function has_call(app: any, call: string): boolean
	return string.find(web_calls(app), call, 1, true) ~= nil
end

local function count_calls(app: any, call: string): number
	local n = 0
	local at = 1
	while true do
		local i = string.find(web_calls(app), call, at, true)
		if not i then
			return n
		end
		n += 1
		at = i + #call
	end
end

local function tab_titles(app: any): string
	if not app:has("#panel-strip") then
		return ""
	end
	local out: { string } = {}
	for name in app:tree("#panel-strip"):gmatch('tab "([^"]+)"') do
		table.insert(out, name)
	end
	return table.concat(out, ",")
end

-- The panel, the page of HTML open in a file tab of it.
local function open_page(): any
	local app = mount("panel_fixture.luau")
	app:resize(900, 640)
	app:frames(10)
	expect(app:error()).to_be_nil()
	expect(app:wait("#panel-empty", { timeout_ms = 15000 })).to_be(true)
	app:command("disk_html")
	app:frames(5)
	app:set_state("test_open_path", "docs/page.html")
	app:command("test.open_file")
	expect(app:wait('"page file:///proj/docs/page.html"', { timeout_ms = 15000 })).to_be(true)
	return app
end

-- The browser tab showing the file, if there is one.
local function browser_tab(app: any): any
	local s = (app:state("panel_sessions") or {})["chat:chat-1"]
	for _, t in (if s then s.tabs else {}) do
		if t.kind == "browser" then
			return t
		end
	end
	return nil
end

test("an HTML file's preview is its page, from what's typed; its source a switch away", function()
	local app = open_page()
	expect(has_call(app, `html:file:///proj/docs/page.html:{#PAGE}`)).to_be(true)
	-- Its source, to edit, and back: the page shows what was typed.
	app:click('role:radio "Source"')
	expect(app:wait('role:textbox "page.html"', { timeout_ms = 15000 })).to_be(true)
	-- (The source fades in: time passes before it takes the keys.)
	app:wait('"never shown, only waited past"', { timeout_ms = 400 })
	app:frames(10)
	app:type('role:textbox "page.html"', "X")
	app:frames(5)
	app:click('role:radio "Preview"')
	expect(eventually(app, function(): boolean
		return has_call(app, `html:file:///proj/docs/page.html:{#PAGE + 1}`)
	end)).to_be(true)
	expect(app:has('#stub-web')).to_be(true)
end)

test("the globe opens the file in a Browser tab, by its path; again, the same tab; anchors and Reload keep it", function()
	local app = open_page()
	app:click('role:button "Open in Browser Tab"')
	expect(eventually(app, function(): boolean
		local t = browser_tab(app)
		return t ~= nil and t.doc == "/proj/docs/page.html"
	end)).to_be(true)
	-- Made from the text, no file's address given; named and addressed as
	-- the file is.
	expect(eventually(app, function(): boolean
		return has_call(app, `html::{#PAGE}`)
	end)).to_be(true)
	expect(tab_titles(app)).to_be("page.html,page.html")
	local field = 'role:combobox "Address"'
	expect(app:wait(field, { timeout_ms = 15000 })).to_be(true)
	expect(eventually(app, function(): boolean
		return app:tree(field):find('="/proj/docs/page.html"', 1, true) ~= nil
	end)).to_be(true)
	local t = browser_tab(app)
	expect(t.url).to_be_nil()
	local page = `panel-browser-{t.id}`
	-- An anchor followed: still the page, still the file's path.
	app:set_state(`test_web_url_{page}`, "about:blank#end")
	app:frames(10)
	expect(app:has(`#panel-browser-start-{t.id}`)).to_be(false)
	expect(app:has('"page about:blank#end"')).to_be(true)
	expect(app:tree(field):find('="/proj/docs/page.html"', 1, true) ~= nil).to_be(true)
	-- Reload makes it again from the text (WebKit's own would be blank).
	local before = count_calls(app, `html::{#PAGE}`)
	app:click(`#panel-browser-bar-{t.id} >> role:button "Reload"`)
	expect(eventually(app, function(): boolean
		return count_calls(app, `html::{#PAGE}`) == before + 1
	end)).to_be(true)
	expect(has_call(app, ",reload")).to_be(false)
	-- The globe again, from the file's tab: the same browser tab, again.
	app:click('#panel-strip >> role:tab "page.html"[1]')
	expect(app:wait('role:button "Open in Browser Tab"', { timeout_ms = 15000 })).to_be(true)
	app:click('role:button "Open in Browser Tab"')
	expect(eventually(app, function(): boolean
		return count_calls(app, `html::{#PAGE}`) == before + 2
	end)).to_be(true)
	expect(tab_titles(app)).to_be("page.html,page.html")
end)

test("a file's Browser tab made again (the scripts reloaded) keeps its page", function()
	local app = open_page()
	app:click('role:button "Open in Browser Tab"')
	expect(eventually(app, function(): boolean
		return has_call(app, `html::{#PAGE}`)
	end)).to_be(true)
	local before = count_calls(app, `html::{#PAGE}`)
	app:set_state("test_panel_generation", (app:state("test_panel_generation") or 1) + 1)
	app:frames(30)
	expect(app:wait('role:combobox "Address"', { timeout_ms = 15000 })).to_be(true)
	expect(eventually(app, function(): boolean
		return app:tree('role:combobox "Address"'):find('="/proj/docs/page.html"', 1, true) ~= nil
	end)).to_be(true)
	-- (The host kept the page as it was: nothing loaded again.)
	expect(count_calls(app, `html::{#PAGE}`)).to_be(before)
end)
