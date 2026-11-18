-- A reply's work (ds/activity) whose steps name pictures, in a chat whose
-- folder is /work, for tests/step_pictures.test.luau: a thought whose
-- Markdown shows two of the folder's and names one outside it, a URL and
-- one in a code span; a long thought with a picture, folded; a Read of a
-- picture in the folder whose result kept no copy (its `named_picture`); a
-- Read of one outside the folder. The pictures are read as the chat's step
-- pictures are (screens/chat/reply_images `for_folder`), each read counted
-- in `test_step_reads` (one line each). Here, beside app.luau, as a mounted
-- script loads modules from its own directory down.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local Activity = require("@ds/activity")
local Lightbox = require("@ds/lightbox")
local ReplyImages = require("@app/screens/chat/reply_images")

local reads = ui.state("test_step_reads", "")

-- What `inside` says of each of these (`path|folder=yes/no`, one a line).
local CASES = {
	{ "/work/a.png", "/work" },
	{ "/work/shots/a.png", "/work/" },
	{ "C:\\work\\a.png", "C:\\Work" },
	{ "C:/work/a.png", "C:\\work" },
	{ "/work", "/work" },
	{ "/workshop/a.png", "/work" },
	{ "/etc/a.png", "/work" },
	{ "/work/../etc/a.png", "/work" },
	{ "/work/./a.png", "/work" },
	{ "a.png", "/work" },
	{ "D:\\work\\a.png", "/work" },
	{ "/work/a.png", "work" },
}
local said = {}
for _, case in CASES do
	table.insert(said, `{case[1]}|{case[2]}={if ReplyImages.inside(case[1], case[2]) then "yes" else "no"}`)
end
ui.state("test_step_inside", table.concat(said, "\n"))

local step = ReplyImages.for_folder(function(): any
	return { id = "chat-1", cwd = "/work" }
end, function(_target: any, path: string): any
	reads:append(path .. "\n")
	local rel = string.match(path, "^/work/(.*)$")
	return {
		after = function(_self: any, ok: (any) -> (), err: (any) -> ())
			if rel then
				ok({ src = rel })
			else
				err("Not in the chat's folder")
			end
		end,
	}
end)

local LONG = {}
for i = 1, 12 do
	table.insert(LONG, `Line {i} of a long thought.`)
end
table.insert(LONG, "![Folded](assets/composer/shell.png)")

local TOOLS = {
	{
		id = "t1",
		kind = "thought",
		verb = "Thought",
		prose = true,
		call = table.concat({
			"Comparing ![The shell](assets/composer/shell.png) with ![The transcript](/work/assets/composer/transcript.png).",
			"Not these: ![Outside](/etc/secret.png), ![Up](../secret.png), ![A URL](https://example.com/x.png), `![Code](assets/code.png)`.",
		}, "\n\n"),
	},
	{ id = "r1", kind = "read", verb = "Read", target = "settings.png", named_picture = { path = "/work/assets/composer/shell.png", name = "settings.png" } },
	{ id = "r2", kind = "read", verb = "Read", target = "private.png", named_picture = { path = "/home/me/private.png", name = "private.png" } },
	{ id = "t2", kind = "thought", verb = "Thought", prose = true, call = table.concat(LONG, "\n\n") },
}

return ui.column {
	w_full = true,
	h_full = true,
	p = "$space.4",
	ui.scroll {
		w_full = true,
		h_full = true,
		Activity {
			key = "work-step-pictures",
			step_pictures = step,
			tools = function(): { any }
				return TOOLS
			end,
		},
	},
	Lightbox.Host(),
}
