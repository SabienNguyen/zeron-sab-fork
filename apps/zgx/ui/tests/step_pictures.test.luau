-- The pictures an agent's steps name, under each opened step as small
-- thumbnails (ds/activity, read by screens/chat/reply_images `for_folder`;
-- ui/step_pictures_fixture.luau): a thought's local pictures under it, a
-- Read's named picture its result kept no copy of under its opened row.
-- Only a picture inside the chat's folder is ever read: one elsewhere on
-- its device, one up out of it, a URL and one in code are never asked for.

test("a step's picture is one only inside the chat's folder, by its words", function()
	local app = mount("step_pictures_fixture.luau")
	app:frames(2)
	expect(app:error()).to_be_nil()
	-- (Each case the fixture asked `inside`: `path|folder=yes/no`.)
	local got = app:state("test_step_inside")
	expect(got).to_be(table.concat({
		"/work/a.png|/work=yes",
		"/work/shots/a.png|/work/=yes",
		"C:\\work\\a.png|C:\\Work=yes",
		"C:/work/a.png|C:\\work=yes",
		"/work|/work=no",
		"/workshop/a.png|/work=no",
		"/etc/a.png|/work=no",
		"/work/../etc/a.png|/work=no",
		"/work/./a.png|/work=no",
		"a.png|/work=no",
		"D:\\work\\a.png|/work=no",
		"/work/a.png|work=no",
	}, "\n"))
end)

local function work(): any
	local app = mount("step_pictures_fixture.luau")
	app:resize(900, 1400)
	app:frames(10)
	expect(app:error()).to_be_nil()
	-- (Settled work folds to its line: open it.)
	app:click('role:button "Thought 2 times · read 2 files"')
	app:frames(30)
	expect(app:error()).to_be_nil()
	return app
end

local function reads(app: any): { string }
	local out = {}
	for line in (app:state("test_step_reads") or ""):gmatch("[^\n]+") do
		table.insert(out, line)
	end
	table.sort(out)
	return out
end

test("folded work reads none of its steps' pictures", function()
	local app = mount("step_pictures_fixture.luau")
	app:resize(900, 1400)
	app:frames(20)
	expect(app:error()).to_be_nil()
	expect(#reads(app)).to_be(0)
end)

test("a thought's pictures in the chat's folder are thumbnails under it; others are never read", function()
	local app = work()
	expect(app:has('"Open The shell"')).to_be(true)
	expect(app:has('"Open The transcript"')).to_be(true)
	for _, name in { "Outside", "Up", "A URL", "Code" } do
		expect(app:has(`"Open {name}"`)).to_be(false)
	end
	-- (Both of the folder's, and nothing else: no Read is open yet, and the
	-- long thought is folded.)
	expect(table.concat(reads(app), ",")).to_be("/work/assets/composer/shell.png,/work/assets/composer/transcript.png")
	app:click('"Open The transcript"')
	app:frames(60)
	local open = app:state("ds/lightbox.luau::open")
	expect(open).never.to_be_nil()
	expect(open.items[open.index].src).to_be("assets/composer/transcript.png")
end)

test("a long thought shows its pictures only once opened", function()
	local app = work()
	expect(app:has('"Open Folded"')).to_be(false)
	local show = app:tree():match('button "(Show all %d+ lines)"')
	expect(show).never.to_be_nil()
	app:click(`role:button "{show}"`)
	app:frames(30)
	expect(app:error()).to_be_nil()
	expect(app:has('"Open Folded"')).to_be(true)
end)

test("a read of a picture in the chat's folder that kept no copy opens to it", function()
	local app = work()
	expect(app:has('"Open settings.png"')).to_be(false)
	app:click('role:button "Read settings.png"')
	app:frames(30)
	expect(app:error()).to_be_nil()
	expect(app:has('"Open settings.png"')).to_be(true)
end)

test("a read of a picture outside the chat's folder shows none and reads nothing", function()
	local app = work()
	app:click('role:button "Read private.png"')
	app:frames(30)
	expect(app:error()).to_be_nil()
	expect(app:has('"Open private.png"')).to_be(false)
	for _, path in reads(app) do
		expect(string.sub(path, 1, 6)).to_be("/work/")
	end
end)
