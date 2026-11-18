-- The lightbox (ds/lightbox) over a message's thumbnails and the composer's
-- chips (ui/lightbox_fixture.luau, a 900 by 600 window): a thumbnail or a
-- chip opens its picture fitted, its thumbnail lifted out; Escape closes it
-- and focus comes back to the thumbnail; ← → go through the set, holding at
-- its ends; zoom walks its steps, fits and goes to its own size; panning
-- never brings the picture's edges inside the window; a click on the
-- picture leaves it open and Close closes it; a small picture is never
-- enlarged to fit; a diagram zoomed past what can be drawn whole has the
-- part on screen drawn on its own over it.

local function open_box(): any
	local app = mount("lightbox_fixture.luau")
	app:resize(900, 600)
	app:frames(10)
	expect(app:error()).to_be_nil()
	return app
end

-- The open stage's view: { s, cx, cy, fitted, w, h, fit, win_w, win_h }.
local function view(app: any): any
	app:command("test.lightbox_view")
	return app:state("test_lightbox_view")
end

local function lifted(app: any): string?
	return app:state("ds/lightbox.luau::lifted")
end

local function line_of(app: any, needle: string): string
	for line in app:tree():gmatch("[^\n]+") do
		if line:find(needle, 1, true) then
			return line
		end
	end
	return ""
end

local function shows(app: any, text: string): boolean
	return app:has(`"{text}"`)
end

local function near(a: number, b: number): boolean
	return math.abs(a - b) < 0.01
end

test("a thumbnail opens its picture fitted, its thumbnail lifted out; Escape closes it and focus comes back to the thumbnail", function()
	local app = open_box()
	app:click('"Open night.jpg"')
	app:frames(90)
	local v = view(app)
	expect(v).never.to_be_nil()
	expect(v.fitted).to_be(true)
	-- 2100 by 700 in 900 less 2 × 40 across: fitted by its width, never
	-- past its own size.
	expect(near(v.s, v.fit)).to_be(true)
	expect(v.s < 1).to_be(true)
	expect(lifted(app)).to_be("m1:3")
	expect(line_of(app, 'dialog "Image viewer"')).never.to_contain("hidden")
	expect(shows(app, "3 of 3")).to_be(true)
	app:key("Escape")
	app:frames(120)
	expect(lifted(app)).to_be_nil()
	expect(app:state("ds/lightbox.luau::open")).to_be_nil()
	expect(line_of(app, 'button "Open night.jpg"')).to_contain("focused")
end)

test("a composer chip opens the chips' pictures; ← → go through them, holding at the ends, the lifted chip following", function()
	local app = open_box()
	app:click('"chip-shell.png"')
	app:frames(90)
	expect(shows(app, "1 of 2")).to_be(true)
	expect(lifted(app)).to_be("chips:c1")
	app:key("ArrowLeft")
	app:frames(30)
	expect(shows(app, "1 of 2")).to_be(true)
	app:key("ArrowRight")
	app:frames(30)
	expect(shows(app, "2 of 2")).to_be(true)
	expect(shows(app, "chip-tall.png")).to_be(true)
	expect(lifted(app)).to_be("chips:c3")
	app:key("ArrowRight")
	app:frames(30)
	expect(shows(app, "2 of 2")).to_be(true)
	-- Another picture: fitted afresh (520 by 1800 in 600 less 40 and 72 down).
	local v = view(app)
	expect(v.fitted).to_be(true)
	expect(near(v.s, (600 - 40 - 72) / 1800)).to_be(true)
	app:key("ArrowLeft")
	app:frames(30)
	expect(shows(app, "1 of 2")).to_be(true)
	expect(lifted(app)).to_be("chips:c1")
end)

test("zooming a picture: ⌘1 is its own size, ⌘+ ⌘− walk the steps, ⌘0 fits; panning never brings its edges inside the window", function()
	local app = open_box()
	app:click('"Open night.jpg"')
	app:frames(90)
	app:key("cmd+1")
	app:frames(60)
	local v = view(app)
	expect(v.fitted).to_be(false)
	expect(near(v.s, 1)).to_be(true)
	expect(shows(app, "100%")).to_be(true)
	-- 2100 by 700 at its own size in a 900 by 600 window: its middle may go
	-- from 900 − 1050 to 1050 across and 600 − 350 to 350 down.
	app:scroll('"Image viewer"', 5000)
	app:frames(5)
	v = view(app)
	expect(near(v.cy, 250)).to_be(true)
	app:scroll('"Image viewer"', -5000)
	app:frames(5)
	v = view(app)
	expect(near(v.cy, 350)).to_be(true)
	expect(v.cx >= 900 - 1050 - 0.01 and v.cx <= 1050 + 0.01).to_be(true)
	app:key("cmd+=")
	app:frames(60)
	expect(near(view(app).s, 1.25)).to_be(true)
	app:key("cmd+-")
	app:key("cmd+-")
	app:frames(60)
	expect(near(view(app).s, 0.75)).to_be(true)
	for _ = 1, 30 do
		app:key("cmd+=")
	end
	app:frames(60)
	expect(near(view(app).s, 16)).to_be(true)
	app:key("cmd+0")
	app:frames(60)
	v = view(app)
	expect(v.fitted).to_be(true)
	expect(near(v.s, v.fit)).to_be(true)
	-- Fitted, it fits whole: nothing to pan.
	local cy = v.cy
	app:scroll('"Image viewer"', 400)
	app:frames(5)
	expect(near(view(app).cy, cy)).to_be(true)
end)

test("a click on the picture doesn't close it; its Close does, focus back on the thumbnail", function()
	local app = open_box()
	app:click('"Open night.jpg"')
	app:frames(90)
	-- (The stage's middle is on the picture. A click beside it closes: the
	-- harness reaches nodes, not points, so that is checked in the gallery.)
	app:click('"Image night.jpg"')
	app:frames(30)
	expect(app:state("ds/lightbox.luau::open")).never.to_be_nil()
	-- (Its controls fade after 1.6 s of a still pointer, and the test's
	-- time is its frames': the pointer moved over the picture brings them
	-- back, as a person's does.)
	app:hover('"Image night.jpg"')
	app:frames(5)
	app:click('role:button "Close"')
	app:frames(120)
	expect(app:state("ds/lightbox.luau::open")).to_be_nil()
	expect(line_of(app, 'button "Open night.jpg"')).to_contain("focused")
end)

test("a picture smaller than the window shows at its own size, never enlarged", function()
	local app = open_box()
	app:click('"Open tiny.png"')
	app:frames(90)
	local v = view(app)
	expect(v.fitted).to_be(true)
	expect(near(v.s, 1)).to_be(true)
	expect(v.w).to_be(64)
end)

test("with motion reduced (Zeron's own setting), it opens and closes as a short crossfade, nothing travelling", function()
	local app = open_box()
	app:set_state("ds/motion.luau::motion_reduce", true)
	app:frames(5)
	app:click('"Open night.jpg"')
	app:frames(2)
	local v = view(app)
	expect(v.morphing).to_be(false)
	-- (A crossfade of at most ds/motion's 120 ms: done within a few frames.)
	app:frames(20)
	v = view(app)
	expect(v.alpha).to_be(1)
	expect(v.lift).to_be(1)
	expect(v.fitted).to_be(true)
	app:key("Escape")
	app:frames(2)
	expect(view(app).morphing).to_be(false)
	app:frames(30)
	expect(app:state("ds/lightbox.luau::open")).to_be_nil()
	expect(line_of(app, 'button "Open night.jpg"')).to_contain("focused")
	app:set_state("ds/motion.luau::motion_reduce", false)
end)

test("with motion as it is, it grows out of the thumbnail", function()
	local app = open_box()
	app:click('"Open night.jpg"')
	app:frames(2)
	expect(view(app).morphing).to_be(true)
	app:frames(60)
	expect(view(app).morphing).to_be(false)
end)

-- The picture's box (its laid-out width and height) in the stage, by name.
local function picture_box(app: any, name: string): (number, number)
	for line in app:tree():gmatch("[^\n]+") do
		if line:find(`img "{name}"`, 1, true) then
			local w, h = line:match(" ([%d.]+)x([%d.]+)")
			return tonumber(w) or 0, tonumber(h) or 0
		end
	end
	return 0, 0
end

test("a picture is never stretched or squeezed as it grows from its thumbnail and goes back, even when its size was said wrong", function()
	local app = open_box()
	-- 868 by 617, said as 400 by 400.
	local want = 868 / 617
	app:click('"Open stale.png"')
	local seen = 0
	for _ = 1, 40 do
		app:frames(1)
		local w, h = picture_box(app, "stale.png")
		if w > 0 and h > 0 then
			seen += 1
			expect(math.abs(w / h - want) / want < 0.01).to_be(true)
		end
	end
	expect(seen > 0).to_be(true)
	app:key("Escape")
	for _ = 1, 40 do
		app:frames(1)
		local w, h = picture_box(app, "stale.png")
		if w > 0 and h > 0 then
			expect(math.abs(w / h - want) / want < 0.01).to_be(true)
		end
	end
end)

test("zooming never leaves a frame without the picture: what the window showed is held under it, exactly where it is", function()
	local app = open_box()
	app:command("test.lightbox_snapshots")
	app:click('"Open night.jpg"')
	app:frames(90)
	local before = view(app)
	expect(before.fitted).to_be(true)
	-- In, step by step, and back out: through every frame of each motion,
	-- a picture of what showed is under the picture, scaled with it, its
	-- corner on the picture's.
	for _, keys in { "cmd+=", "cmd+=", "cmd+-", "cmd+0" } do
		app:key(keys)
		for _ = 1, 20 do
			app:frames(1)
			local v = view(app)
			expect(v.under).never.to_be_nil()
			expect(math.abs(v.under.corner_x - v.under.pic_x) < 1).to_be(true)
			expect(math.abs(v.under.corner_y - v.under.pic_y) < 1).to_be(true)
			expect(v.under.w > 0 and v.under.h > 0).to_be(true)
		end
	end
end)

-- A detail's view box, as numbers.
local function view_box(d: any): { number }
	local v = {}
	for part in string.gmatch(d.view_box, "[^%s]+") do
		table.insert(v, tonumber(part) :: number)
	end
	return v
end

-- The detail covers the picture's part of the window, on whole points, at
-- the size it shows, and looks at that part of the document (520 by 1800,
-- the picture's own size: one unit a pixel).
local function expect_covers(v: any)
	local d = v.detail
	expect(d).never.to_be_nil()
	local pw, ph = v.w * v.s, v.h * v.s
	local px0, py0 = v.cx - pw / 2, v.cy - ph / 2
	expect(d.x).to_be(math.round(d.x))
	expect(d.y).to_be(math.round(d.y))
	expect(d.x <= math.max(0, px0) + 0.5).to_be(true)
	expect(d.y <= math.max(0, py0) + 0.5).to_be(true)
	expect(d.x + d.w >= math.min(v.win_w, px0 + pw) - 0.5).to_be(true)
	expect(d.y + d.h >= math.min(v.win_h, py0 + ph) - 0.5).to_be(true)
	expect(near(d.laid_w, d.w) and near(d.laid_h, d.h)).to_be(true)
	local b = view_box(d)
	expect(math.abs(b[1] - (d.x - px0) / v.s) < 0.01).to_be(true)
	expect(math.abs(b[2] - (d.y - py0) / v.s) < 0.01).to_be(true)
	expect(math.abs(b[3] - d.w / v.s) < 0.01).to_be(true)
	expect(math.abs(b[4] - d.h / v.s) < 0.01).to_be(true)
	-- Only about the window, not the whole diagram.
	expect(d.w * d.h < pw * ph).to_be(true)
	expect(d.code).to_contain(`viewBox="{d.view_box}"`)
	expect(d.code).never.to_contain('viewBox="0 0 520 1800"')
	expect(d.code).to_contain('rx="12"')
end

test("a diagram zoomed past what can be drawn whole draws the part on screen on its own, sharp, and lets it go zoomed out", function()
	local app = open_box()
	app:click('"Open diagram.svg"')
	app:frames(90)
	expect(view(app).detail).to_be_nil()
	-- Its own size (520 by 1800, 3.7M pixels at 2x) is drawn whole.
	app:key("cmd+1")
	app:frames(60)
	expect(view(app).detail).to_be_nil()
	-- 150%: 8.4M pixels, past it.
	app:key("cmd+=")
	app:key("cmd+=")
	app:frames(60)
	local v = view(app)
	expect(near(v.s, 1.5)).to_be(true)
	expect_covers(v)
	-- A short pan stays inside its margin: the same drawing, moved.
	local code = v.detail.code
	app:scroll('"Image viewer"', 60)
	app:frames(30)
	v = view(app)
	expect(v.detail.code).to_be(code)
	expect_covers(v)
	-- A long one goes past it: drawn again where the window is now.
	app:scroll('"Image viewer"', 1200)
	app:frames(30)
	v = view(app)
	expect(v.detail.code).never.to_be(code)
	expect_covers(v)
	-- Further in, and it follows the zoom.
	code = v.detail.code
	app:key("cmd+=")
	app:frames(60)
	v = view(app)
	expect(near(v.s, 2)).to_be(true)
	expect(v.detail.code).never.to_be(code)
	expect_covers(v)
	-- Fitted, the whole is enough again.
	app:key("cmd+0")
	app:frames(60)
	expect(view(app).fitted).to_be(true)
	expect(view(app).detail).to_be_nil()
	expect(app:error()).to_be_nil()
end)

test("a picture with no vectors, or vectors with no size to map, is never drawn in parts", function()
	local app = open_box()
	app:click('"Open night.jpg"')
	app:frames(90)
	app:key("cmd+1")
	app:key("cmd+=")
	app:key("cmd+=")
	app:key("cmd+=")
	app:frames(60)
	expect(view(app).s >= 2).to_be(true)
	expect(view(app).detail).to_be_nil()
	app:key("Escape")
	app:frames(120)
	app:click('"Open unsized.svg"')
	app:frames(90)
	app:key("cmd+1")
	app:key("cmd+=")
	app:key("cmd+=")
	app:key("cmd+=")
	app:frames(60)
	expect(view(app).s >= 2).to_be(true)
	expect(view(app).detail).to_be_nil()
	expect(app:error()).to_be_nil()
end)
