var e=`# What’s New in Golem Docs

## 0.33.1 - Unreleased

### Fixed

- A document opened from a view-only share or a public link no longer slides sideways when its page already fits the width of the window.

## 0.33.0 - 2026-11-03

### Added

- File › Share can turn on a public link: anyone with it can read the document and download it without signing in, and Copy Embed Code puts it on another website. Reset Link makes a new address and turning it off stops it working.

### Fixed

- In the desktop app, a document changed in the vault folder while the app was closed no longer opens, disappears and opens again at launch: the change goes into it once it has opened.
- The Can edit / Can view choice in File › Share now matches the email box and buttons beside it.

## 0.32.0 - 2026-11-03

### Added

- Excel workbooks (.xlsx) open in a new spreadsheet editor: type into cells, insert and delete rows and columns, resize them, freeze panes, merge cells, add and rename sheets, copy and paste with other apps, find, and undo. Large workbooks of a million rows open, scroll and zoom smoothly, out to 0.1%, and open again in a moment from this browser. Zoomed out, text too small to read shows as grey lines just where it sits, so a sheet still looks like itself. Files save back as .xlsx that Excel opens as they were.
- Formulas in workbooks calculate as you type, as Excel calculates them: hundreds of Excel's functions, dynamic arrays that spill into the cells below, and references that follow rows and columns as they are inserted and deleted.
- Workbook cells can be formatted: bold, italic, underline, fonts and sizes, text and fill colours and alignment from the Format menu and the toolbar, and number formats, decimal places, borders and wrapped text from the Sheet menu. A Sheet toolbar shows while a workbook is open, with buttons for currency, percent, decimal places, borders, merging, wrapping, alignment, sorting and filtering.
- Workbooks can be sorted (Sheet › Sort A to Z, Sort Z to A), Edit › Replace replaces text in cells, and double-clicking the edge of a column header fits the column to its contents.
- Drag the corner of a workbook selection to fill cells: numbers, dates, months, days and numbered text carry on as a series, and formulas are copied with their references moved.
- Sheet › Filter adds filter buttons to a workbook's header row: pick the values to show, or sort by the column.
- Sheet › Insert Table turns a block of cells into a table, with filter buttons on its header row. A formula typed into an empty column of a table fills the whole column, and rows inserted into the table get it too.
- Tables in workbooks show in their styles, and formulas can use structured references such as Sales[Amount] and [@Price].
- Cells with a drop-down list of choices in a workbook show an arrow to pick from (or Alt+Down), and a value a cell's rules don't allow is turned away with the rule's message.
- Notes in workbooks show as a red mark in the cell's corner, and rest the pointer on the cell to read them; they stay with their cells as rows and columns are inserted and deleted.
- Sheet › Protection protects a workbook sheet, with a password if you like: its locked cells can't be changed until Unprotect Sheet, and Lock Cells and Unlock Cells choose which cells stay open. Sheets protected in Excel stay protected, and their passwords work in both.
- Sheet › New Note adds a note to a workbook cell (or changes the one it has) in a card beside the cell, and Sheet › Delete Note takes it away.
- Cells copied within a workbook paste with their formulas and formatting, and Sheet › Paste Special pastes only values, formulas or formatting, or rows and columns swapped.
- Charts in workbooks show over their cells: column and bar, line, area, pie, doughnut and scatter charts, with their titles, legends and axes. They change as their data does, and move with their cells when rows and columns are inserted or deleted. Insert › Chart makes a chart of the selected cells; click a chart to pick it, drag it to move it or its corners to resize it, press Delete to remove it, or right-click it to change its type, legend or title, or its series and categories with Select Data. A chart changed this way keeps the formatting it had in Excel.
- Sparklines in workbooks show in their cells: lines, columns and win/loss, with their high, low and negative points marked, and they move with their cells. Insert › Sparklines adds them to the selected rows, and Clear Sparklines takes them away.
- Insert › PivotTable makes a pivot table of the selected cells on a new sheet: choose its rows, columns, values (sum, count, average and more) and filters in the field list. Right-click a pivot table to change its fields or refresh it from its data. Pivot tables are laid out as Excel lays them out. They are drawn in their pivot style (shaded headers and grand totals, bold outer items, indented inner ones), as in Excel. The field list sets a pivot table's layout (compact, outline or tabular), where its subtotals go and which grand totals it shows, and a pivot table's Row Labels and Column Labels buttons sort a field or choose which of its items to show.
- CSV and TSV files open in the spreadsheet grid at every size, and are saved back as the same files: unchanged rows stay exactly as they were. A file of millions of rows shows its first rows at once and fills in as the rest is read.
- File › Convert To › Excel Workbook turns a document's first table into a workbook, and a workbook converts to and from CSV and TSV without leaving the grid.
- A workbook's card in File › Open shows the top of its sheet.
- A workbook in your account is edited on one device at a time: open it on another computer or share it, and the others see it read-only, updated each time it is saved, and can take it over.
- Sheet › What-If Analysis › Goal Seek finds the value of a cell that brings a formula to the value you want.
- Sheet › What-If Analysis › Data Table shows a formula's results for a row and a column of inputs, and data tables in Excel workbooks now update as you edit.
- Sheet › What-If Analysis › Scenario Manager keeps named sets of input values: show one, add, edit or delete them, and make a summary sheet of what each gives. Excel's scenarios are kept.
- Sheet › What-If Analysis › Solver makes a formula as large or small as it can be, or brings it to a value, by changing cells within constraints, with the Simplex LP and GRG Nonlinear methods and whole-number cells. Solver models saved in Excel open here, and the other way round.
- File › Print and Print Preview print a workbook's sheet as Excel does: its paper, margins, scale or fit to pages, print area, title rows and columns repeated on each page, page breaks, gridlines, headings, header and footer. The workbook stays open in the grid. Charts and sparklines print too. File › Page Setup sets them, Sheet › Print Area sets the selection as the print area, and Sheet › Page Breaks adds or removes page breaks.
- Conditional formatting in workbooks shows as in Excel: highlighted cells, colour scales, data bars and icon sets, recalculated as the values change.
- Sheet › Group groups rows or columns in a workbook: collapse and expand them from the outline beside the headers, or show a level with its numbered buttons. Groups made in Excel show the same way.
- Emoji show in color everywhere: documents of every format, text files, workbook cells and charts, and PDF, picture and SVG downloads and printed sheets. Families, flags, keycaps and skin tones each show as one emoji, and the caret, selection and Backspace treat each as one character. The emoji images load only when a document uses one.
- On a phone or tablet, a long press selects the word under your finger, and a double tap does too; keep your finger down and drag to select more. Dragging near the top or bottom of the screen scrolls the document as you select.
- On a phone or tablet, a selection has a handle at each end: drag one to change where the selection starts or ends.
- The website’s footer has a Sponsor link, for anyone who wants to support Golem Docs through GitHub Sponsors.

### Changed

- On a phone or tablet, tapping inside a word puts the cursor at the end of the word, so you don't have to aim between two letters, and a double tap no longer needs to land on exactly the same spot.
- Someone you share a document with as “Can view” (File › Share) now opens it in a simple read-only viewer: pages, zoom, find, and select and copy, with nothing to edit. They see the latest saved version, and it catches up by itself while you keep working; once you stop sharing it with them (or they leave it), it disappears from their screen. Choosing between “Can edit” and “Can view” in the Share dialog is now a pair of buttons.

### Fixed

- Workbooks saved by programs that don't calculate formulas showed empty cells where the formulas were; they are now calculated when opened.
- An Excel workbook synced from the desktop folder opened as a document holding only its first sheet's table; it now opens as the workbook.
- Pasting a hundred thousand rows or more into a sheet failed.
- View › Full Screen works on phones, where the browser has no full screen of its own: the menus and toolbars go away, and a button in the corner brings them back. It is also in the phone's menu.
- Copying and pasting into another app keeps paragraph indents that come from a paragraph’s style, and copying a long document (Edit › Select All) keeps its formatting.
- In the desktop app, files that changed in your vault folder no longer download again when you open them. The app keeps the copy it just synced, so they open straight away.

## 0.31.0 - 2026-11-02

### Added

- Tools › Thesaurus (Shift+F7): look up words of the same meaning as the selected word, or the word at the caret, and replace it with one. Double-click a word in the list to look that one up in turn; Previous goes back. The thesaurus is in English.
- Right-click a word for its synonyms: pick one to put it in place of the word, or choose Thesaurus… to see them all.

## 0.30.1 - 2026-10-31

### Changed

- The website’s About page now says Golem Docs is open source, and that you can ask for a feature or send the code for it on GitHub.

### Fixed

- Taking over a PDF or picture someone else was editing could leave you with the copy from before their last change, so the next save would lose it. You now always get what they had.

## 0.30.0 - 2026-10-31

### Added

- Golem Docs is open source. A new Open source page on the website explains why, and how you can use the app, run your own copy, or build on the code. The home page now leads with the editor and shows how to keep a second brain in plain files, with links between notes.

### Changed

- Menus, toolbars, dialogs and the rest of the app now use Atkinson Hyperlegible Next, a font designed so letters that look alike (I, l and 1; O and 0) are easy to tell apart. It looks the same on every computer and phone. Russian, Japanese, Korean and Chinese still use your system’s font.

### Fixed

- Saving a Markdown file no longer drops the titles of links and pictures (\`![alt](picture.png "title")\`) or rewrites their addresses: an address is saved the way it was written, with or without \`<\` \`>\`. Titles also become the link’s tooltip in Word.
- A folder deleted or renamed in your vault folder (by another program, or from the command line) now disappears from your documents too, once nothing is left in it, instead of staying behind empty.

## 0.29.0 - 2026-10-31

### Added

- Right-clicking in the documents view or the Files sidebar now offers the last three types of document you used (New Markdown, say) straight away, so you need not open the New document list. The full list is still there below them.

### Changed

- The toolbar’s New button, and New in the documents view, now show the icon of the type of document they make (the one you used last), and their tooltip names it.

### Fixed

- On a phone or tablet, opening the documents view no longer puts the cursor in the search box, so the keyboard stays closed until you tap it.

## 0.28.0 - 2026-10-26

### Added

- The website’s ai and Pricing pages now explain how to use your documents on several computers for free: put your Golem Docs folder in iCloud Drive, Dropbox, OneDrive or Google Drive and choose it on each computer.

### Changed

- When a document was changed in two places before they could catch up (offline on a laptop, say, while an ai app edited it in your vault folder on another computer), the changes are now merged if they are in different paragraphs, instead of being kept as a conflicted copy. Changes to the same paragraph are still kept apart, and Version History has each side as it was.
- A document you have open shows a change saved from your vault folder on another computer (by an ai app working there, say) as soon as it is saved, highlighted for a moment, and your next edit builds on it instead of being kept as a conflicted copy.

### Fixed

- A vault folder kept in iCloud Drive, Dropbox or a similar cloud drive and opened in Golem Docs on a second computer no longer has its files deleted. Each computer now keeps its own record of the folder. If a sync would delete most of the folder’s files, it now waits for you to confirm, as it already did when most files were missing.

## 0.27.0 - 2026-10-24

### Added

- In the desktop app, Cmd+W (Ctrl+W on Windows) closes the open document, like File › Close.

### Changed

- The website has a new look, in every language: separate pages for using Golem Docs with your ai, every format it opens, pricing, a journal of what changed, and downloads, with a home page that shows the editor and an ai at work instead of describing them.
- The Files sidebar is now open when you start, on a screen wide enough for it. Close it, or choose View › Files Sidebar, and it stays the way you left it.

## 0.26.0 - 2026-10-24

### Changed

- PDFs are shown in the fonts they were made with, the ones embedded in the file, instead of a stand-in such as Arial or Times New Roman, so a LaTeX paper looks as it does printed, sharp at any zoom.
- Desktop app: when a document's file changes in your vault folder while it is open, only what changed is updated, in place, without the page flickering. The new text is highlighted for a moment, and the caret and what you were looking at stay where they were.

### Fixed

- PDF Fill In and Edit PDF: a mark, shape or text box placed right after putting another one down is no longer lost.
- Text in a PDF that doesn't embed its fonts is shown in a serif font, bold or monospaced as the font's name says, for LaTeX's own fonts too, instead of all in Arial.

## 0.25.0 - 2026-10-24

### Added

- File › Close puts the open document away, leaving no document open, with buttons to start a new one or open another. The app stays that way the next time you come back.

### Changed

- Deleting the document you have open now leaves no document open, instead of opening the Welcome document, and so does going back to a document that is no longer there (instead of an empty new one). Welcome is only shown on your first visit.

## 0.24.0 - 2026-10-23

### Changed

- Desktop and iPhone apps: your vault folder now holds every document as a real file, including PDFs (with your annotations, form fields and edits written in), pictures (with their layers drawn in), drawings, EPUB and MHTML. A file changed in the folder replaces the document, and your account keeps the earlier version.
- The built-in AI assistant is gone. To use AI with your documents, point an AI agent such as Claude Code, Codex or Cursor at your documents folder in the desktop app; what it changes syncs back like any other edit.
- The subscription is now $3 a month, and includes 25 GB of storage, version history included. History uses whatever space your documents leave: when the account fills up, the oldest automatic versions make room first. Versions you name are kept. The home page and the welcome document say so.
- When your account is full, a save that would add to it is kept on your device and the sync button says how to make room: delete documents or empty the trash. Edits that don’t make a document bigger always save.

### Fixed

- Desktop app: large files sync to and from your folder much faster (after you update the app).

## 0.23.3 - 2026-10-23

### Added

- Documents can be renamed from their card on the Documents screen and from their menu in the Files sidebar, not only while they are open.
- In the Files sidebar on a touch screen, holding a finger on a document or folder opens its menu.

### Fixed

- On a phone, a document can now be renamed: the menu shows its name at the top (tap it to rename it), and File › Rename is in the menu.
- The buttons under a document's card on the Documents screen are now all the same width.

## 0.23.2 - 2026-10-23

### Fixed

- Desktop app: documents uploaded as Markdown, text or other non-Word files sync to your folder again, instead of stopping with “not a ZIP file”.

## 0.23.1 - 2026-10-22

### Fixed

- The home page and its translations no longer redirect to an address ending in a slash, so search engines can index them.

## 0.23.0 - 2026-10-18

### Added

- Right-clicking in a table now opens a menu to insert rows above or below, insert columns to the left or right, and delete rows, columns or the whole table.

### Fixed

- After the computer woke from sleep, a document in your account could stop taking any typing until it was reopened.

## 0.22.2 - 2026-10-15

### Changed

- The Vault Folder dialog now shows what a sync is doing (reading the account, comparing, copying file 3 of 12), how many files and documents it found, and every file that failed with its reason, instead of only "Syncing…".

### Fixed

- The desktop app's vault no longer refuses every file with "expected the file's bytes" when the web view sends data the slow way.
- A vault sync no longer starts over every few seconds after it hit errors, and a document the account has no file for is reported instead of counted as a failure.
- A vault sync now stops after five files in a row fail, and shows the reason, instead of working through every file.
- A vault sync that got stuck (a file conversion that never finished, or uploads that never completed) now reports the problem instead of saying "Syncing…" for ever.
- Clicking a task list's check box no longer selects the line (or lines) around it.
- In Flow view, a document no longer lays out twice when it opens (the text re-wrapped a moment after appearing).
- Text in the default font no longer pops in a moment after a document opens: its fonts now start loading as the app starts.

## 0.22.1 - 2026-10-14

### Fixed
- The Windows installer builds again (0.22.0 had none: a check of the new phone file-opening code failed there). Nothing else changed.

## 0.22.0 - 2026-10-14

### Added
- On a phone or tablet, files opened in Golem Docs from Files, Mail or another app (Open in…, Share) open as documents, and several at once are imported.
- Tools › Options › View › “Allow scrolling half a screen past the end of the document” lets you scroll the last page up to the middle of the screen so you can keep typing there. It is on by default and never scrolls for you.

### Fixed
- On Android, the keyboard no longer opens when you tap the menu button or when a document opens (it opens when you tap the document), and the menu stays open when the window changes height.
- The web (Flow) view no longer shows page breaks: page and section breaks are ignored and the text runs on without gaps. Page breaks only show in the print view.
- The assistant moves up out of the way of messages and progress cards at the bottom of the screen.

## 0.21.1 - 2026-10-14

### Fixed
- In the desktop app, an update of the app is announced once: while a newer version of the app is on offer, the note about a new version of the site is not shown as well. A site that is newer than the installed app no longer reports the vault folder as unusable, and Show in Finder and Open Privacy Settings say to update the app instead of failing.

## 0.21.0 - 2026-10-14

### Added
- Options open with ⌘, (Ctrl+, elsewhere); in the Mac desktop app they are also in the app menu.
- Desktop app on a Mac: Tools › Options can also show the menus in the window, alongside the system menu bar (off by default).
- A Requests section on the home page: there is no plugin store, missing features are built into the app, and it says how to ask for one. The feature cards are fewer, and the smaller extras have their own section.

### Changed
- On a phone or tablet, File › Download and Download All (ZIP) open the system share sheet (Save to Files, AirDrop, Mail and other apps) instead of a browser download.
- In the desktop app, the save indicator at the top right shows the folder that keeps your documents as files, with a link to show it in Finder (or File Explorer), or to choose one.
- The home page now says that a file type Golem Docs doesn’t support, or a missing feature, will be added.
- The home page says that Golem Docs is free in the desktop app and, when they arrive, the mobile apps, not only in the browser.

### Fixed
- Renaming a document from the title at the top: the box is now just a little wider than the name, and grows as you type, instead of filling the whole bar.
- Pinching on a trackpad zooms the document in the Mac desktop app (and in Safari).
- The desktop app now checks that it may use the vault folder before every sync, which makes macOS ask for permission. If the folder can’t be read or written, it says so (in a notice, in Vault Folder and in the save indicator) and offers Try Again and, on a Mac, a button to open Privacy settings. Before, a folder it couldn’t read failed silently.
- In the Mac desktop app, the window’s close, minimize and zoom buttons no longer cover the Documents heading.
- On the home page, no two neighboring sections share a background color any more.
- On a phone, tapping the document puts the caret down and opens the keyboard on the first tap (iOS closed it again a moment later), and tapping the AI assistant opens it on the first tap instead of the second.
- On a phone, the menu bar and status bar are no longer hidden by the notch, the rounded corners or the home indicator, and the assistant no longer sits on top of the status bar or the typing bar.
- On a touch screen, the delete button on a page in the page sidebar is always shown, so a tap on a page no longer needs a second tap.

## 0.20.0 - 2026-10-13

### Added
- The desktop app now updates itself: it looks for a new version when it opens and every few hours, asks before installing, and Help › Check for Updates looks right away.
- A Download section on the home page (golemdocs.com/download) with the desktop app's installers for Mac, Windows and Linux, the one for your computer first.

## 0.19.1 - 2026-10-12

### Fixed
- www.golemdocs.com now opens golemdocs.com instead of failing to load.

### Changed
- On a phone, the Open view starts with smaller cards (three across), and the buttons under each document or folder are one even row that no longer wraps or changes size with the icons.

## 0.19.0 - 2026-10-12

### Added
- A page left open now notices when Golem Docs has been updated and offers to reload into the new version, saving what is open first.

## 0.18.0 - 2026-10-12

### Added
- A desktop app for Mac, Windows and Linux. File › Vault Folder keeps a copy of your documents as ordinary files in a folder on your computer (Markdown, text, Word and the other text formats stay in their own format) and sends changes you make to those files back to your account. If a document changed in both places, the file in your folder wins, and your account keeps the earlier version in its history. Files you delete from the folder go to your account's trash for 30 days. PDFs, pictures and drawings stay in your account. You do not need an account: signed out, the folder is kept in step with the documents saved on this computer in the same way (deleted files go to the trash there), and when you sign in later the folder carries on with your account. A document open in the editor is reloaded when its file is changed in the folder (unless you have unsaved edits in it). The first time you open the desktop app, it opens the Vault Folder dialog so you can choose the folder (or skip it and choose later).
- In the Mac desktop app the menus are in the menu bar at the top of the screen, where Mac users expect them, and the menu bar inside the window is gone. The document name and the controls that were on the right now share the window's title bar, which also drags the window.

### Fixed
- Renaming a document by clicking its name now selects the whole name in the desktop app and in Safari, as in Chrome, so typing replaces it. Select All, Undo and Redo work in the name box and in dialog text boxes in the Mac desktop app.

## 0.17.2 - 2026-10-12

### Fixed
- Reopening a document now puts the caret and any selected text back exactly where they were, even in a window of another size or another view, and remembers where you left it however you opened it (from the Documents screen, the Files Sidebar or its address). The scroll position is kept too when you moved it by typing or with the keyboard, not only by scrolling.
- The status bar no longer shows the Wrap Long Lines button for documents, or the Print Layout, Flow and Page Grid buttons for plain text files, where they did nothing.

## 0.17.1 - 2026-10-09

### Fixed
- The Down and Up arrow keys no longer skip short or empty lines when the caret is near the right end of a long line.

## 0.17.0 - 2026-10-09

### Added
- Resting the pointer on a link in a document opens a small form over it with the link's text and address, to change them (OK), take the link off (Remove Link) or open it. Insert › Hyperlink (Ctrl/Cmd+K) opens the same form at the caret, instead of a separate dialog.
- New guides on the website for converting a PDF to Word, converting Markdown to Word and back, redacting a PDF, and opening Word files without Word.

### Changed
- A fresher look for dialogs, buttons and text fields: flat surfaces with a slim accent line along the top of each dialog, a plain title and close button, and the main action (OK, Save, Apply) filled with the theme's color. The menu bar and toolbars are flatter too.
- Pinching or Ctrl/Cmd+scrolling over the toolbars and panels no longer zooms the whole page, which broke the layout; it still zooms the document over the page itself.
- The buttons and search box on the Documents screen now look like the ones in dialogs.
- Markdown documents are edited as formatted text again: Tools › Show Markdown at the Caret is gone. Autoformat as You Type stays, and now works on every line, so \`* \` starts a bullet list and \`*word*\` and \`**word**\` make italic and bold as you type.
- Large documents opened from your account appear as soon as they have downloaded, instead of waiting for the copy kept in this browser to be written first, and the status bar shows how much of the download has arrived.
- Large documents get to their first page a little sooner: the default fonts load while the document downloads, and the check for which characters a document needs is faster.
- Links to Golem Docs shared in chat apps and social networks now show a large preview card.

### Fixed
- On a phone or tablet, a document or folder on the Documents screen opens with one tap instead of two.
- Very long documents no longer sit behind the loading screen until every page is laid out: the first pages appear as soon as they are ready, and the rest fill in behind them. Opening a long document is also about twice as fast, because the page sidebar and spelling underlines no longer redo all their work after every batch of pages.
- Copying or cutting text now keeps its formatting (bold, italics, links, headings, lists) when pasted into other apps.
- Clicking a task-list check box in a Markdown document checks or unchecks it again, even when its Markdown is shown, and no longer moves the cursor.
- In a large text file, the “Replaced n matches” message after Replace All stays on screen instead of being replaced by “No matches” a moment later.

## 0.16.0 - 2026-10-08

### Added
- Pictures in a document can be picked with a click: a frame with handles appears, Delete removes the picture, a drag moves it (in line with the text, or to a new place on the page for a floating one), the corner handles resize it, and a double click opens it in the image editor (Done puts the changed picture back in the document, Cancel leaves it as it was). Right-clicking a picture offers Cut, Copy (the picture itself goes on the clipboard), Delete and Edit Picture. The assistant can do the same with its new text_pictures tool.
- HEIC and HEIF photos (the iPhone's picture format) open, can be inserted into documents (Insert › Picture, drag and drop, the assistant's picture button) and are shown to the assistant. They are kept as high-quality JPEGs. Safari reads them itself; other browsers load a decoder the first time you use one, so nothing extra is downloaded otherwise.

### Changed
- Error and warning messages that pop up at the bottom of the window are now bold white text on a red background in every theme, so they are easier to notice.

### Fixed
- A "(conflicted copy)" is only made when the two versions really differ; if both are the same (a blank document, say), nothing extra is created.
- Dragging a picture from your computer onto a Markdown note (it is embedded as a file of its own) or a PDF page (it is placed where you drop it) now offers to insert it, as it does in Word documents.
- Dragging a picture from the Files panel onto a document asks whether to insert the picture or a link to it (a PDF takes the picture), instead of always making a link.
- Converting a PDF to Word keeps the text where the PDF had it: a page whose text starts far down the page no longer starts at the top (over the pictures), headings keep their bold and the text its color, and a page's last line is not pushed onto a new page.
- Converting a PDF to Word keeps the running header and footer text (dates, titles, web addresses, page numbers, which are now page fields) as the document's header and footer instead of dropping it.
- Pictures made of vector shapes (QR codes, logos) are drawn sharply in the converted document, not blurred.
- Drawings in a document (such as the vector art from a converted PDF) are drawn from their SVG, so they stay sharp however far you zoom in, and zooming stays smooth. Tiling patterns in an SVG are drawn too.
- A double letter such as the “ll” in “All” is no longer taken for bold text when converting a PDF.

## 0.15.0 - 2026-10-08

### Fixed
- Converting a PDF to Word keeps vector drawings (logos, charts, line art, QR codes and barcodes) as pictures, drawn as SVG so they stay sharp, with their gradients, patterns, clipping, dashes and line joins. They used to be left out. A PDF with no text on it also keeps its pictures now.
- Converting a PDF to Word keeps pictures that are turned or mirrored (including mirrored JPEGs), and small ones. They used to be dropped or shown the wrong way round.
- PDFs with black-and-white fax (CCITT) scans now show and convert those images.

### Changed
- On a phone, drop-downs (New, font, size, colors, the “…” buttons and more) open as sheets from the bottom of the screen, above the keyboard, instead of small pop-ups, and New is one button that lists the document types.
- On a phone, the top toolbar is for files (New, Open, Undo, Redo, Find, Share, and “…” for the rest), and while you type, a formatting bar (Bold, Italic, Underline, lists, Link, Image, Comment, and “…” for the rest) sits right above the keyboard.

## 0.14.0 - 2026-10-06

### Added
- File › Version History keeps the earlier versions of every document as you edit it, so you can look back at how it was. They are kept in this browser, and in your account too if you have a subscription. Opening a version makes a new document and leaves the one you are in as it is, so nothing is ever overwritten. You can also save the document as it is now under a name (a named version is kept longer), rename a version and delete it. Anyone who can edit a shared document can open its versions. The assistant can look at the history, keep a version and open one.
- File › Trash (and a Trash button on the Documents screen): documents you delete are kept for 30 days, whether they were in your account or only in this browser (a document you made while signed out, say), and you can bring one back (one from your account returns unshared) or delete it for good. The assistant can list the trash and restore from it.

### Changed
- Deleting a document now moves it to the trash instead of deleting it for good. Signing out, or Clear Local Data, still deletes what is kept in this browser.

## 0.13.0 - 2026-10-06

### Added
- Autocorrect as you type, as in Word and Google Docs: straight quotes become smart quotes (in the language of the paragraph), \`--\` an em dash, \` - \` an en dash, \`...\` an ellipsis, \`(c)\`, \`(r)\`, \`(tm)\`, \`1/2\` and \`->\` become ©, ®, ™, ½ and →, \`1st\` gets a raised suffix, the first letter of a sentence is capitalized, \`TWo\` becomes \`Two\`, and a web address followed by a space becomes a link. Backspace right after a correction gives back what you typed. Common English typing mistakes (\`teh\`, \`recieve\`, \`dont\`) are corrected when you finish the word, and Tools › Options › Autocorrect as You Type › Edit List… holds your own replacements (\`omw\` becomes \`on my way\`), kept on this device and in your account when you are signed in. Each rule can be turned off in Tools › Options › Autocorrect as You Type.
- View › Full Screen now also hides the menu bar, toolbars, ruler and status bar, leaving only the document, and a short message reminds you that Esc leaves full screen and brings them back. Tools › Options › View can keep the menu bar showing in full screen.
- Rest the pointer on a red or blue squiggle for a moment and its menu opens (suggestions, Ignore, Add to Dictionary), with no right click needed. It does not move the caret or select the word, so you can keep typing; typing, scrolling or moving away closes it.
- After a new version is released, a note centered at the bottom of the window offers a button to What’s New. Close it to hide it until the next release, or choose Don’t show again to turn it off.

### Changed
- Red and blue squiggles are calmer while you type: the word you have just finished is underlined after a short pause instead of the moment you press space.

### Fixed
- The assistant no longer says some documents are “still being prepared for searching” when they are PDFs or pictures, which can never be searched by meaning.
- A web address now also becomes a link when you press Enter after it, or right away when you paste it on its own (it already did when you pressed space), if the links autocorrect is on.
- Ignore All and Ignore Rule in the spelling and grammar menu and dialog now stay ignored in that document after you reload or reopen it (and on your other devices when you are signed in), instead of the squiggle coming back. They no longer carry over to other documents.
- Typing no longer stops working after you right-click (or Control-click on a Mac) a word with a squiggle and close the menu.

## 0.12.0 - 2026-10-06

### Added
- View › Toolbars › Show Toolbars hides or shows all the toolbars under the menu bar at once, for more room to write. The choice is kept, and each toolbar keeps its own setting for when they come back. The assistant can do it too.

### Changed
- PDFs and pictures in your account are now edited by one person at a time, whether it is someone you shared with or you on another device. Whoever opened it first edits it with every tool, including moving, deleting and rotating PDF pages, adding large pictures to a PDF, and cropping, resizing, rotating and changing the colours of a picture, none of which worked before while others had it open. Everyone else sees it update each time it is saved, and can choose Take over to edit it themselves. If you come back to one with changes that never reached your account while someone else is editing it, your changes are kept as a separate “(conflicted copy)” rather than lost or saved over theirs. Documents, text files and drawings are still edited by everyone at once. The assistant can say who is editing and take over when you ask.

### Fixed
- Opening a document in your account no longer waits on the editing-together connection when it is slow: the document opens after a few seconds at most and connects in the background.
- Losing the connection while you are alone in a document (on a plane, say) no longer stops you editing it: it is saved as usual, and connects again for editing together the next time it saves.
- Changing a drawing or a picture in your account no longer drops it out of editing together for a moment, so someone opening it at that moment (or your other device) no longer ends up with a separate copy.

## 0.11.0 - 2026-10-04

### Changed
- File › Share no longer has Get Link. The link only worked between tabs of the same browser, so it could not be sent to anyone else. To edit together, share the document with people by email (they edit or view it live), or open it in your account on another device. Old \`?room=\` links open your last document instead.

## 0.10.1 - 2026-10-04

### Fixed
- Converting a document to plain text (File › Convert To) says again what the text file leaves out, instead of only “ready”.

## 0.10.0 - 2026-10-04

### Added
- Drawing tablets (Wacom and other pens, or a stylus): in pictures, the Pen and Highlighter draw a thicker line the harder you press, and turning the pen over to its eraser end erases what it passes over, whatever tool is picked. In drawings, a Pencil line drawn with a pen follows how hard you press too (it becomes the Pen Pressure stroke; a closed loop stays an even line), and the Pen Pressure stroke follows the pen instead of its ready-made swell. Tools › Options › Drawing can turn pen pressure off. Fast strokes keep every point the pen reports, so they come out smooth rather than angular.
- Large text files in your account can now be edited together, like documents: everyone sees the others' typing and carets as it happens, and Ctrl/Cmd+Z only takes back your own changes. Files up to 16 MB can be shared; while others are editing, Replace All handles up to 20,000 matches, and the shared file is kept as UTF-8.
- Plain text files (.txt) open in the large text editor are spell checked as you type: misspelled words are underlined, and right-clicking one offers suggestions, Ignore All and Add to Dictionary, as in documents, and Tools › Spelling and Grammar (F7) steps through the problems of a text file. Logs, code and data files are not checked as you type.
- Find and Replace in documents has Whole word and Regular expression options (with \`$1\` in the replacement), as the large text editor already did.
- Large text files (8 MB or more, such as logs) can now be edited, not only looked at: click to put the caret in a line, type, select with the mouse or keyboard (Shift, Ctrl/Cmd and Alt as usual), cut, copy and paste, and undo and redo (Ctrl/Cmd+Z). Edit › Replace replaces one match at a time or everywhere in the file in one step (with regular expressions and \`$1\`), and File › Open Selection as Document opens the selected lines as an ordinary document you can format and convert. An edited file saves itself in this browser (and in your account, up to 50 MB) like any document, keeps its encoding (UTF-8, or UTF-16; a Windows-1252 file is saved as UTF-8), and Download gives it back with your changes. While showing only matching lines you can change text within a line but not add or remove lines. The assistant can read, search, replace in, and edit the lines of a large text file, and undo.
- The picture editor is much closer to a full drawing program: its toolbar lists (line style, thickness, opacity) now look like the drawing editor’s, and a shape’s fill type and gradient are in its Fill Color popup, as in drawings. There is no Undo/Done bar any more: undo with Ctrl/Cmd+Z, and press Enter or Escape to put the tools down. The pointer now shows the tool you picked (a pencil, highlighter, eraser, eyedropper, paint bucket, or a crosshair), the Pen and Highlighter show a circle the size of the line, and the keys are Photoshop’s and can be changed in Help › Keyboard Shortcuts, in a new Picture Tools section: V Move, B Pen, Shift+B Highlighter, E Eraser, L Line, Shift+L Arrow, U Rectangle, Shift+U Ellipse, Alt+U Shapes, T Text, Shift+T Bubble, Shift+I Numbered Marker, R Redact, I Eyedropper, G Paint Bucket, C Crop, [ and ] for a thinner or thicker line, Ctrl/Cmd+A, D, J, G and [ ] to select all, deselect, duplicate, group and reorder, and F7 for Layers.
- Pictures have a Layers panel (Picture › Layers, F7), like the vector editor’s: everything you draw is a layer, with an eye to hide it, a padlock to lock it, double-click to rename, drag to reorder or to put it inside a group, New Layer (new shapes go into the layer you picked), Group and Ungroup, Duplicate and Delete, and each layer’s opacity and blend mode (Multiply, Screen, Overlay and more; on a group it applies to everything inside). The picture itself is the Background row at the bottom. Each tool you pick gets a layer of its own at once (so you can select, hide or rename it before drawing; one nothing was drawn on goes away again), named for the tool (Pen 1, Rectangle 2…) and shown closed so the list stays short; the same tool keeps drawing into it, and picking a layer first draws into that one. A layer opens and closes with its arrow. Click a layer to pick it, Shift+click to pick every layer from the last one clicked, and Ctrl/Cmd+click to add or remove one. The layers panel is now the same one in drawings and pictures.
- The Move tool picks shapes and layers (click, Shift+click to add, or drag a box around them), moves them, resizes them by their handles (Shift keeps the proportions, Alt resizes from the middle), nudges them with the arrow keys (Shift for ten pixels) and deletes them with Delete. Drawing with Alt starts a rectangle, ellipse or line from its middle.
- New Draw tools: Shapes (the vector editor’s ready-made shapes: stars, hearts, arrows, polygons and more), an Eyedropper that picks a colour from the picture and what is drawn on it (Alt-click sets the fill colour), and a Paint Bucket that floods an area of the picture with a colour or gradient (with a tolerance, and an option to fill only the connected area). A fill is a layer like any shape: it finds its area in what you see under it (the picture and the layers below it, so it stops at the edges of shapes drawn before it, but not of ones drawn after) and does not change the picture’s pixels, so it can be hidden, moved, erased or undone, is shared with others editing the picture, and the picture does not reload.
- Drawn shapes have a separate fill colour, a solid, linear or radial gradient fill, opacity, and the same line styles as drawings (dashes, and brushes such as tapered, calligraphic, dry brush, chalk, dots and stars), and can be filled without an outline. Picked shapes can be rotated by the knob above their box (Shift turns in 15° steps), and each shape tool has its own pointer.
- New blending brushes for pictures, as in a paint program: Smudge (R) drags colours along as if with a finger, the Mixer Brush (Alt+B) paints with the colour you picked and mixes it into the colours it passes over like wet paint, the Blur Brush (Shift+R) and Sharpen Brush (Alt+R) soften or sharpen where you paint, and Dodge (O) and Burn (Shift+O) lighten or darken. A Strength setting says how strongly they work, the line thickness sets the size of the brush, and the Redact tool is now X. Each stroke is a layer like any other: it works on what is under it (the picture and the layers below), so it can be hidden, moved, erased, undone and shared, and the picture’s pixels are not changed until you flatten it.
- Picture › Background lays a colour or gradient behind the picture, showing wherever the picture is see-through, and Picture › Canvas Size makes the canvas bigger or smaller around the picture, anywhere you like (nine anchor positions), filling the new area with a colour or gradient.
- The assistant can use all of these on pictures: ready-made shapes with fills, gradients, opacity and dashes, layers (group, rename, hide, lock, reorder, copy, opacity and blend), the paint bucket, the background and the canvas size.
- In the Documents view, drag a box across the background to select every file and folder it touches (hold Ctrl, Cmd or Shift to add to what is selected); the list scrolls when you drag near its top or bottom edge.
- New documents are made in the type of the last document you used, and a drop-down arrow beside New in the toolbar lets you pick another type (Word, Markdown, plain text, HTML, LaTeX and more), listing the types you used most recently first. The assistant can choose the type too.
- The assistant can use the new drawing and editing-together features: it can insert a drawing in a Word or Markdown document, open a drawing in the text to draw in it with its drawing tools and put it back, say who is editing a document. It draws on pictures as layers (list, move and erase them, or put them into the picture), and is told, rather than failing silently, when others are editing and a change isn’t possible (a picture’s own pixels, a PDF’s pages, a drawing someone else holds). Starting to share a document is still yours to do, in File › Share.
- Pictures can now be drawn on together: share a picture and everyone sees the lines, shapes, text and markers the others draw, as they draw them, and can draw at the same time. What you draw is a layer over the picture, not part of its pixels: it stays where it is until you move, erase or undo it (undo takes back only your own), it is saved with the picture, and it is put into the picture when you save it as a picture or change the picture itself (crop, rotate, resize, colours). While others are editing a picture those changes to the picture itself aren’t available.
- PDFs can now be marked up together: share a PDF and everyone sees the highlights, notes, shapes, form entries and edits the others make as they make them. Undo takes back only your own changes. Moving, deleting, rotating or adding pages isn’t available while others are in the PDF yet.
- Vector drawings can now be edited together: share a drawing and everyone sees each change as it is made. Everyone edits at once, with no waiting: changes are merged shape by shape, so two people can draw, move and delete different shapes together. (If two people change the very same shape at the same moment, the later change wins.) Drawings in a shared Word document work as before.
- Pictures and drawings now work while a document is being edited together: add one and everyone in the document sees it, and a drawing can be edited in place by anyone. Pictures are kept with the document’s room as it is edited and are in the saved file once it ends. (A drawing embedded in a shared Markdown note can’t be added or edited yet.)
- Live editing together now works for every document you edit as text, not only Word: share a Markdown, HTML, OpenDocument, RTF, plain text or CSV document and the people who join edit it as that format, with its front matter and settings. Pictures, drawings and PDF pages still can’t be shared live.
- Vector drawings in Markdown notes: Insert › Drawing stores a new drawing of its own and embeds it (\`![[Drawing.svg]]\`). An embedded drawing, new or one you already have, can be double-clicked to edit it in place with the full vector tools; the changes are saved into the drawing, and the note keeps only the embed.
- Vector drawings in Word documents: Insert › Drawing puts a blank drawing in the text, and an SVG file inserted as a picture comes in as a drawing too. Double-click a drawing to edit it in place: the drawing is lit, the rest of the page is dimmed and the whole Drawing toolbar and menu are there. Click outside it, or press Escape with nothing picked, to put it back in the text (one Undo takes the whole edit back). The drawing is saved in the .docx as an SVG beside its picture, the way Word keeps SVG pictures.
- The vector editor has a Canvas tool (C): drag a corner or side of the page to make it bigger or smaller. The shapes stay where they are, a size shows as you drag, Shift keeps the proportions of a corner and Alt grows the page around its centre. The assistant already does this with Canvas Size.
- In the vector editor, the drawing’s size in the status bar is a button: click it to open Drawing › Canvas Size.
- The vector editor has Illustrator’s keys: V selects, M draws a rectangle, L an ellipse, \\\\ a line, P uses the pen, N the pencil, B the nudge brush and T adds text; Ctrl/Cmd+G groups (with Shift, ungroups), Ctrl/Cmd+[ and ] move a shape back and forward (with Shift, to the back and front), Ctrl/Cmd+7 makes a clipping mask, Ctrl/Cmd+U toggles snapping and F7 shows the layers. Help › Keyboard Shortcuts lists them in their own Vector Editor section, where each can be changed to a single letter, digit or symbol key.
- Edit › Search All Documents (Ctrl/Cmd+Shift+F) finds words and #tags in every document you have: this browser’s copies, your account’s and what others shared. Every word must be in the document, a partly typed word counts, and choosing a result opens the document at the paragraph that matched. Put a phrase in quotes (\`"exact words"\`) to find it in one paragraph, and \`-word\` or \`-#tag\` to leave out documents that have it. PDFs are searched by their text too. The Documents screen’s search box now looks inside documents too, and the assistant searches the same way. The first search of a large library prepares each document once; with sync, the account keeps that work so your other computers don’t repeat it.
- Tags: type \`#project\` and then a space, comma, Enter or other punctuation (not a full stop), in any document, and it becomes a tag, drawn in its own colour: a link (Ctrl/Cmd+click) that opens a search of every document with that tag. In Markdown a tag is saved as plain \`#project\`; \`#work\` also finds \`#work/plans\`. With nothing typed, the search lists your tags to pick from. The assistant can find tagged documents and tag text.
- Your settings now follow your account: appearance and dark pages, language, keyboard shortcuts, autoformat, Markdown at the caret, spelling and grammar options, your personal dictionary, the view, formatting marks, the ruler and the status bar are the same on every computer you sign in on. A phone or tablet keeps settings of its own. Toolbar layout, panel sizes and what each document remembers stay on the device. Needs sync.
- View › Ruler shows a ruler above and beside the page in view, in inches (centimetres in other languages), with the margins marked; the assistant can turn it on and off. Drag a margin’s edge on either ruler, or the indent markers on the top one (first line, hanging, left, right), to change them.
- Obsidian-style links in Markdown: \`[[Note]]\`, \`[[Note#Heading]]\` and \`[[Note|label]]\` link to the stored document of that name, are made as you type them in Markdown (the space after \`]]\` turns one into a link), and are written back the same way. Links follow folders like Obsidian’s: \`[[Notes/Plan]]\` finds Plan in a folder called Notes, \`[[/Notes/Plan]]\` from the top level, and \`[[../Plan]]\` relative to the note’s own folder; a plain \`[[Plan]]\` prefers the note beside it. Following a link to a note that doesn’t exist yet makes its folders too.
- Renaming a document, or renaming or moving a folder or document, now updates the links to it in your other documents (\`[[Plan]]\`, \`[[Notes/Plan]]\`, \`[[./Plan]]\`, embeds and links to a heading), so they keep working. A link keeps its style and its own label, and a note someone else is editing live is left alone and reported. Search indexes are rebuilt once to record each document’s links.
- Drag a document from the Files sidebar onto the open document to make a link to it where you drop it (\`[[Note]]\` in Markdown). It works in any format that can hold links, and uses the folder path when the name alone would be ambiguous.
- Obsidian embeds in Markdown: \`![[map.png]]\` shows the picture in place (\`![[map.png|300]]\` sets its width), \`![[Note]]\` shows that note’s text, and \`![[Note#Heading]]\` shows the part under a heading. They are looked up from your documents each time the note opens, and saved back as \`![[ ]]\`, never as copies.
- Two new pages on the website: how to import files, folders, and ZIP archives, and how to import an Obsidian vault.
- File › Import Folder, and an Import folder button on the Documents screen, import a folder from your computer (with its subfolders) the way a ZIP is imported.
- The welcome document ends with a short note about the developer and what Golem Docs is for.
- Importing files, folders or a ZIP whose names match documents you already have now asks what to do: overwrite, keep both or skip, with a checkbox to answer once for all the rest. A file identical to the document already there is skipped without asking, which makes syncing a folder again quick. Documents that are only in your account are compared too (downloaded only when their size matches).
- SVG and compressed SVG (.svgz) files open as vector drawings of their own kind, staying sharp at any zoom, listed under Vector rather than Images in Convert To. Convert any document to SVG and its text stays text; convert a drawing to PDF and it stays vector; to PNG, JPEG, WebP or Word and it becomes a picture at full resolution.
- Vector drawings can be edited: a Drawing menu and toolbar select, move, resize and rotate shapes, draw rectangles, ellipses, lines, pen paths and text, edit a path's points (double-click it), set fill and outline, group, reorder, duplicate and delete, with undo and redo.
- The Pencil tool shows the selected brush as you draw, not only once you let go.
- The Rectangle, Ellipse and Line tools show the shape filled and outlined with your chosen colours as you drag, not only a thin outline.
- Drawing › Shape adds a Shapes tool with 23 ready-made shapes (rounded rectangle, triangles, polygons, stars, heart, arrows, plus, moon, cloud, speech bubble, lightning bolt): pick one from the list beside the tool on the Drawing toolbar and drag it out. The assistant can add them too.
- Drawing strokes beyond a plain line: dashes, tapered and swelling width profiles, calligraphic pens, dry brush, chalk, dry and frayed ink, charcoal, patterns along the path, scattered shapes, and arrowheads, chosen from the Stroke Style list; a stroke keeps its path, so the path can still be reshaped.
- In the vector editor, Unite, Intersect, Subtract and Exclude combine shapes into one; Make Clipping Mask cuts shapes to the one on top; and fills can be linear or radial gradients, with points and stops you drag on the drawing.
- Vector drawings have a Layers panel to hide, lock, rename, select and drag shapes into a new order or into groups, and shapes snap to each other and the page with guide lines (Drawing › Snap to Objects).
- Clip, the assistant, can read and change vector drawings, and a drawing's gradients stay gradients when it is converted to PDF.
- Vector drawings can have opacity masks (the topmost shape's brightness shows or hides the rest) and guides you add from the Drawing menu, drag into place, and double-click to remove; guides are kept in the file and shapes snap to them.
- Drawing › Canvas Size sets a vector drawing's page size (in proportion or not), lets the shapes stay or scale with it, adds a background color, and fits the page to the artwork.
- In the vector editor, Ctrl or Cmd plus a click picks a shape inside a group, double-clicking a group goes into it so that clicks and rubber bands pick its children (Escape comes back out), and the layers list can pick anything.
- In the vector editor, Ctrl+Alt+click picks the shape underneath the one picked, and again for the next one down, round to the top. In the layers list, hidden shapes show a red eye and locked ones an orange padlock, so they stand out.
- The vector editor has a Pencil for free drawing: draw with the pointer and it turns the stroke into a smooth path with just a few points, keeps corners sharp, and closes a loop you finish where you began.
- The vector editor has a Nudge tool: drag it across a drawing and the lines under the round brush bend and move with the pointer, the most at the middle and none at the edge; locked and hidden shapes stay put.
- The Nudge tool has a tool-options strip at the top of the drawing with Size and Strength sliders (strength is how much of your drag reaches the lines, from 10% to 100%); \`[\` and \`]\` make the brush smaller and larger. Both settings follow your account, and the assistant’s \`nudge_lines\` takes a strength too.
- The layers list has New Layer and New Group buttons (also in the Drawing menu and toolbar). A layer is a top-level group that Inkscape also reads as a layer, and new shapes are drawn onto the layer you picked or made last; New Group adds an empty group, or groups what is picked.
- Tools › Options has a Drawing section with options for the vector editor: whether a finished pencil stroke is selected (also under Drawing › Select After Pencil), whether the Select tool comes back after you draw a shape or pen path, and how much the pencil smooths a stroke. They follow your account, and the assistant can change them.
- Drawing › Show Guides hides the guides in a vector drawing without removing them (shapes then stop snapping to them), and Drawing › Lock Guides stops them being dragged or removed by accident. Adding a guide shows them again. Both are also in Tools › Options, and the assistant can set them.
- The import progress card shows an estimate of the time left.
- The website has a new section showing War and Peace, all 1,006 pages, zooming in Page Grid.

### Changed
- Plain text files (.txt) now open in the text editor at every size, as large ones already did: they open fast, show line numbers, are spell checked, can be edited together, and keep their encoding. Print and Convert To first open the file as a document (files up to 5 MB). Switching a Word document to Plain Text is now final: the formatting is not kept for switching back. Library and assistant search read text files line by line, and the Insert menu's Date and Time and Symbol work in them.
- The large text editor's status line is now the status bar documents have (View › Status Bar): line count and size, the line in view, indexing progress, the zoom list, and a button to wrap long lines. Typing, input methods, Backspace and Enter from a phone's keyboard, copy, cut and paste now work the same in documents and in the large text editor, and so do the word keys (Ctrl+arrows on Windows, Option+arrows on a Mac).
- Moving by word (Ctrl/Alt+arrows, double-click) in large text files now follows the rules of documents: a word keeps its apostrophes, Chinese and Japanese words are found, and moving right lands on the start of the next word.
- Saving a big document (2 MB or more) is much faster, here and in your account: it is saved and uploaded in pieces, and only the pieces you changed are written and sent, so a small edit to a 50 MB file sends a few megabytes instead of all of it. Smaller documents, and downloads, work as before.
- JetBrains Mono, a new font in the font list, is now the monospace font for plain text documents, Markdown code and code blocks, in place of Courier New (a document that names Courier New keeps it). Its lines are a little taller.
- Large text files are drawn like documents: in JetBrains Mono, on the same canvas, so they zoom smoothly (Ctrl/Cmd+wheel, pinch, the zoom menu and box) and pan the same way as a document. Lines wrap at the width of the window at 100% zoom, so zooming never rewraps them.
- The home page no longer says “too” in two cards in a row, and its picture card now lists what the picture editor does beyond cropping and resizing: rotating, color adjustments, drawing, text and numbered markers.
- The vector editor’s Nudge brush now pushes only the selected shapes when something is selected; with nothing selected it still pushes every line it touches.
- In the vector editor, the Fill Color popup now holds the whole fill: choose Solid, Linear or Radial at its top, and a gradient shows a bar of its colors with a marker for each stop. Click a marker to pick it, drag it to move it (the end stops too, even past their neighbors), click the bar to add a stop, type its position, Delete Stop (any stop, as long as two are left), or Reverse the gradient; the palette colors the picked stop and the popup stays open. With nothing selected, it sets up the gradient the next shapes you draw are filled with. The separate Fill Type list is gone from the toolbar, and No Fill and No Stroke now show a crossed-out swatch.
- Tools › Show Markdown at the Caret is now on by default, alongside Autoformat as You Type; both can be turned off in the Tools menu.
- The website and the welcome document now speak in the developer's own voice, and the pricing explains that the subscription covers server and AI costs.
- File › Clear Local Data now signs you out too (warning first about changes not yet in your account), so the next person on a shared computer isn’t signed in as you.

### Fixed
- Text files no longer open with empty space above the first line: a short file starts at the top of the window instead of in the middle, and scrolling stops at the first line.
- Edit › Copy was off in the large text viewer even with lines selected; it now copies them.
- Pictures and drawings can be inserted into a document in your account again: when you were signed in with sync, Insert › Picture did nothing even with nobody else in the document.
- A document shared with you that its owner saved in the app’s own format (and any Markdown or other non-Word document) now opens in its own format, and your edits to it keep it. Before, it failed to open or became plain Word.
- Sharing a document live no longer turns the account’s copy of a Markdown (or other non-Word) document into plain Word: it keeps its format, front matter and settings, and a copy already damaged this way opens as Word instead of as garbled text.
- The assistant’s search by meaning in your account’s documents could not read documents the app had saved, so it never found them; it reads them now.
- Ctrl/Cmd+click on a link now follows it when Show Markdown at the Caret is on, instead of only putting the caret in the link's Markdown.
- Following a link to a document that doesn't exist asks in the app's own dialog, not the browser's, whether to create it.
- Color pop-ups (font color, highlight, fill, outline) no longer have a wide empty strip beside the swatches, and the vector Fill pop-up now lets you pick solid, linear or radial and explains how to edit a gradient.
- Importing a ZIP no longer starts with the progress bar a quarter full; it fills steadily and never moves back.
- The delete and ZIP import progress cards now fill to the end and stay a moment before closing, instead of vanishing part way.
- Moving, resizing or drawing on a picture or drawing while someone else edits it no longer undoes their changes when you let go.
- Changes to a drawing inside a document are no longer lost when you open another document while still editing it.
- Select All in a drawing no longer picks shapes on locked or hidden layers, shapes in a locked layer can't be picked from the Layers panel, and new shapes no longer land in a locked or hidden layer of a drawing or picture.
- Keys pressed in a dialog no longer reach the document behind it: Delete in Keyboard Shortcuts no longer deletes the selected shape, and Enter on a comment or a dialog button no longer adds a paragraph.
- The assistant no longer changes a document you switched to while it was working on another, and it finds shapes inside layers of a drawing.
- A conflicted copy of a conflicted copy keeps the name "(conflicted copy)" instead of adding another.
- Shapes drawn on a picture are saved: before, a picture whose only change was what you drew on it could lose it when you closed or reloaded the page.
- Page thumbnails, in the Pages panel and on the Documents screen, show the shapes drawn on a picture and a drawing as you edit it, instead of a blank or old page.
- Clip stays over the document instead of covering the bottom of the Layers panel.
- Dark mode: toolbar, menu and file icons are drawn in light ink so they can be seen; comment cards, the comments bar, the math dialog, the Documents search box and folder names, search results and highlights no longer show light text on a light background.
- Selections, handles, focus rings and drop markers follow the theme's color (orange in Golem) instead of always being blue.
- Selection handles look the same when moving shapes in a drawing or picture, cropping, and editing a PDF; menus, option bars, toasts and notices share one corner and shadow, and the crop shape list matches the other boxes.

## 0.9.0 - 2026-10-03

### Added
- Admin › Analytics shows how many unique people use Golem Docs, how many are new or signed in recently, how many of them pay, and how the number grew month by month.
- A Collapse All button at the top of the Files sidebar closes every folder at once.
- Buttons at the right end of the status bar switch between Print Layout, Flow and Page Grid, the one in use shown pressed.
- In the status bar, clicking the page opens Go To, the word count opens Word Count, and the zoom opens the list of zoom sizes.

### Changed
- The status bar is shown from the start; View › Status Bar still hides it, and that choice is remembered.
- Page Grid is now a view of its own beside Print Layout and Flow (View › Page Grid, or Alt+G to go back to the view before it), and Normal view is renamed Flow.
- On a PDF, the Page Sidebar lets you drag pages into a new order and delete them in Fill In too, without switching to Edit PDF.
- The close buttons of the sidebars and of every dialog look the same and line up.

### Fixed
- Print, Print Preview and PDF downloads use the document's own fonts (Calibri, Arial, Times New Roman and the rest) instead of plain Helvetica, Times or Courier, and keep characters such as curly quotes, dashes and accented letters.
- Clip, the assistant, sits above the status bar instead of covering it.
- Typing, a picture or a save made the moment a document opens is no longer lost or refused.
- The Documents screen and the Files sidebar always show the same documents and folders, and a save or upload no longer makes the app ask the account for its list several times over.

## 0.8.0 - 2026-10-02

### Added
- Dropping several files, or a folder, from your computer imports them all, each in the folder it was in (a ZIP among them goes in a folder of its own name). One file still opens as before.
- The Files sidebar can be reached with Tab, and Left Arrow on a document moves to the folder it is in.
- A collaborative document whose connection was lost rejoins by itself once you are back online.
- The AI assistant can open one of your documents, start a new blank one (in a folder, with a name), and download all your documents as a ZIP.
- Dark Pages (View › Color Scheme) turns the pages, and their thumbnails in the Page Sidebar and on the Documents screen, dark with light letters along with the rest of the app; pictures in the open document keep their colors.

### Fixed
- Opening a document no longer flashes a lighter background before its pages appear, and the loading box only shows when a document takes a moment to open.
- The Page Sidebar stays open, emptied, while another document opens, instead of closing and opening again.
- Two devices saving the same document at the same moment no longer lose one of the saves, and a save that arrives just after the document was deleted no longer leaves its file behind.
- A document deleted with its folder can no longer be downloaded afterwards, by you or by the people it was shared with.
- Moving many documents, or importing a ZIP with many folders, no longer pauses your account (and others) for minutes on the server's request limit.
- Renaming a document while editing it with others now reaches your account.
- A ZIP with one password-protected, damaged or unusually compressed file now imports everything else instead of failing; names made by Windows' own ZIP folders keep their accents.
- A large text file imported in a ZIP opens in the large-file viewer instead of freezing the tab.
- One part of a ZIP import that the server refuses no longer sends the rest of the import the slow way.
- The name you are typing for a new folder (in the Files sidebar or on the Documents screen) is no longer reset when the list refreshes.
- Moving ticked items while searching no longer pulls folders and documents out of a ticked folder they are in.
- Opening one document and then quickly another no longer lets the first replace the second when it finishes loading, and Download names the file after the document it came from.
- The assistant no longer carries on editing a different document if you open one while it works: it stops and says so.
- An editing error in the layout engine is now reported instead of the pages silently no longer updating.
- A document deleted from your account on another device is no longer removed here if you edited it while the list was loading.
- Documents shown before your account's list loads now include ones you just uploaded, and a shared document you left no longer reappears.
- Signing in with a different account no longer mixes in links to the previous account's documents.
- A change made on the Documents screen now shows in the Files sidebar too, and the other way round.
- A move or rename made while your document list was loading is no longer undone in the list shown the next time the app starts.
- Signed in without a subscription, a document no longer tries to join a shared editing session it can't be part of.
- Two rare error messages are now in your language instead of English.
- The Files sidebar stays quick after importing thousands of documents.
- Importing a ZIP into your account no longer makes some documents twice when you switch to another tab, go back online or edit while it runs.
- Editing a document someone shared with you no longer renames their copy (to the name of the next document you open, or with a .docx ending).
- Renaming a folder in one place and moving it in another (the Files sidebar and the Documents screen) no longer undoes the first change.
- Deleting the open document while offline no longer leaves it to be saved again later as a second copy.
- The documents in a folder you just deleted no longer come back at the top level of your lists.
- Going back to a document you just left no longer opens it without your last few edits.
- Signing out now removes the list of your account's documents and folders kept in the browser, and signs out the app's other open tabs too.
- A new document, import or dropped file no longer lands in a folder you chose earlier for something else (after cancelling Import, say).
- In a ZIP, a file with a very long name is no longer skipped as unsupported, an accented name made on a Mac no longer makes a second folder beside the one you have, and a GIF, BMP or SVG no longer gets the same name as a PNG beside it.
- A document in your account now joins its collaboration room even when it takes a while to open, so edits made at the same time as others aren't saved as a separate copy.
- A collaborative document you return to with Back or Forward rejoins its room instead of quietly no longer sharing your edits.
- A malformed edit sent to a shared document can no longer stop the document from ever saving again.
- Your account no longer becomes unusable when an update to its storage is interrupted part way.

## 0.7.1 - 2026-10-02

### Added
- Opening a document now shows it in the Files sidebar, with the folders it is in opened and scrolled into view.

### Changed
- File › New started from inside a document now puts the new document in the same folder as the one you are in.

### Fixed
- Renaming a document from the name in the menu bar now updates the Files sidebar and document list right away, and the new name is kept for the next visit (also for documents shared with you).

## 0.7.0 - 2026-10-02

### Added
- Deleting a large number of documents or a big folder shows a progress card.
- Tools › Show Markdown at the Caret (on by default; turn it off in the Tools menu): in a Markdown document, the line the caret is in shows its Markdown prefix (\`## \`, \`- [ ] \`), and the bold, italic, strikethrough, code or link text the caret is in shows its \`**\`, \`*\`, \`~~\`, \`\` \` \`\` or \`[](url)\` in gray, while the rest stays formatted. Edit the Markdown, then move the caret away to see it formatted. Showing and hiding the Markdown is not an edit, so Undo only sees what you changed.

### Fixed
- Deleting a folder with thousands of documents is far faster and no longer stalls with no progress.
- Deleting or moving a very large folder (more than 500 items) no longer fails with an error about too many ids.
- While a ZIP is importing, documents no longer show at the top level before jumping into their folders.
- Importing a ZIP that contains empty files no longer leaves them failing to upload to your account.
- Importing a large ZIP into your account no longer fails part way with server errors: documents are sent in batches, the lists refresh less often while it runs, and if the server asks for a pause the import waits and carries on.
- Moving a folder or document, or making a folder, in the sidebar no longer flickers back to how it was before showing the change.
- Saving a Word document no longer rewrites its comments when they haven't changed, so Word's own comment details are kept exactly as they were.

## 0.6.0 - 2026-10-02

### Added
- Format as you type: typing \`# \`, \`- \`, \`1. \`, \`> \`, \`[ ] \` at the start of a line makes a heading, list, quote or check box list; \`**bold**\`, \`*italic*\`, \`\` \`code\` \`\`, \`~~strike~~\` and \`[text](url)\` followed by a space become formatted text; \`---\` or three backticks then Enter make a line or a code block. Backspace right after takes it back. Turn it off under Tools › Autoformat as You Type.
- Markdown task lists (\`- [ ]\` and \`- [x]\`) show a check box in place of the bullet, and are saved back the same way. Click a box to check or uncheck it. Check box lists are on the toolbar and in the Insert menu.
- Each kind of file (Word, Markdown, PDF, HTML, spreadsheet, image and more) has its own small icon before its name on the Documents screen and in the Files Sidebar.
- Reopening a document puts you back where you left off: the same scroll position, zoom and caret, remembered on this computer.
- The Documents screen now opens in the folder of the document you have open, instead of at the top level.
- Drop a file from your computer on a folder in the Files Sidebar or on the Documents screen to open it and file it in that folder. Dropped on a document in the sidebar, it goes in that document's folder; dropped elsewhere on the Documents screen, in the folder you are looking at.
- View › Files Sidebar pulls out a tree of your folders and documents beside the editor, so you can switch documents without the Documents screen. Click a folder to open or close it, and a document to open it; the one you are editing is marked. It stays closed until you choose it, and the choice is remembered.
- Both the Page Sidebar and the Files Sidebar can be resized by dragging their right edge (or with the arrow keys once it is focused; double-click resets it). The page thumbnails grow and shrink with the Page Sidebar. Widths are remembered.
- File › Import (or dropping a file on the window) accepts a ZIP: its documents go in the same folders as in the ZIP, making the folders that are missing and reusing the ones you already have. Nothing is overwritten; a document whose name is taken in its folder gets (2), (3) and so on. Imported from inside a folder on the Documents screen, the ZIP's contents go in that folder.
- Importing a ZIP shows its progress in a card at the bottom right, with a progress bar and a Cancel button. Signed in, the bar counts each document's upload to your account too, and the documents appear on the Documents screen and in the Files Sidebar as they are added. It doesn't block the window, and cancelling keeps what was already imported.
- The Documents screen has an icon-size slider at the top right. The document icons now start smaller, with less space between rows, and the size you choose is remembered. It now covers the whole window, so the menu bar and toolbars are hidden while it is open.
- The action buttons on files and folders in the Documents screen (Share, Move, Rename, Delete, Leave) are now icons; point at one to see its name.
- The Documents screen no longer shows your account name or the Sign in, Sign out and Subscribe buttons; they are in the menus.
- Clip can now organize your files: list your documents and folders, make and rename folders, move documents and folders, and delete them, many at a time in a single step.
- The Edit PDF and Fill In panel has a minimize button (a chevron) that rolls it up to just its drag handle and the button; click it again to open it. It stays the way you left it.
- The AI assistant can now look through all your other documents (the ones in your account, saved in this browser, or shared with you): it can find the ones about a topic, read them, and copy paragraphs from them into the document you are working on, with their headings and lists.
- The AI assistant can now look at pictures. Drop a picture on its bubble, paste one, or use the ＋ button to attach up to four, then ask about it. It can also look at a picture in the document you have open, or one saved in your library (“what is in the second picture?”). Pictures are scaled down before they are sent.
- The browser's Back and Forward buttons now move between the documents you opened and the Documents screen, instead of leaving the app.

### Changed
- Documents on the Documents screen that have no picture yet show the Golem face's page text in place of grey lines.
- Deleting, moving or renaming a document or folder on the Documents screen or in the Files Sidebar shows at once instead of after it is saved; if saving fails, it comes back with the reason.
- Moving or deleting many documents and folders at once is now one request to your account instead of one per item, so it is much faster.
- Opening a document from your account is faster, especially with many files: the server no longer looks the document up over the network on every open, a document starts downloading as soon as you rest the pointer on it, and the lists refresh after it appears instead of before.
- Your list of documents and folders in the account loads much faster: the server keeps a copy of it instead of querying your storage each time, and the document, shared and folder lists are now fetched together.
- Deleting a folder now deletes every file and folder inside it too, and a confirmation dialog lists everything that will be deleted, in a scrollable box. The same dialog now asks before any document or folder is deleted, on the Documents screen and in the Files Sidebar.
- Deleting the document you have open now switches to the Welcome document, instead of leaving the deleted one on screen.
- File › Download All keeps your folders: documents are zipped inside the same folder structure.

### Fixed
- Right-clicking in the Files Sidebar and choosing New folder now leaves the new folder's name ready to type. It used to lose the name box straight away, as the list refreshed.
- Deleting several ticked folders or documents on the Documents screen now deletes them, instead of them coming back a moment later.
- Deleted folders and documents no longer pop back into the Documents screen or Files Sidebar for a while before disappearing, and a document that was uploading as you deleted it is no longer brought back as a new one.

## 0.5.1 - 2026-10-01

### Fixed
- Deleting a folder that has no documents in it (only subfolders) no longer fails with "The number of keys in the request must be between 1 and 1000".
- The Files Sidebar now shows your saved folders and documents the moment the app opens, instead of only this browser's files until the account has answered.
- A document name that fits in the menu bar is no longer cut short with “…”.

## 0.5.0 - 2026-10-01

### Added
- View › Status Bar (off until you turn it on) shows a strip under the document with facts about it, which depend on the kind of document: the page and word count of a text document (and how many words are selected), the rows and columns of a CSV or TSV file, the page of a PDF, the size in pixels of a picture, and the zoom. The AI assistant can turn it on and off too.
- The Documents screen's New document, Import and New folder are now buttons in its header, and Esc closes it from anywhere.
- Select several documents and folders on the Documents screen (tick boxes, Ctrl/Cmd-click, Shift-click for a range, or Ctrl/Cmd+A) to move or delete them all at once.
- Folders: organize your documents in nested folders on the Documents screen. Make a folder, rename it, and move documents and folders into it by dragging or with Move…. Deleting a folder keeps what was in it. Folders sync with your account, and a document you start from inside a folder is saved there.
- A folder's icon now shows small thumbnails of the first documents inside it.
- The form and PDF editing tools panel can be dragged by its top edge to move it out of the way.
- Pressing Enter on an empty bullet or numbered item now ends the list instead of adding another empty item.
- Open, edit and save five more formats: Typst (.typ), DocBook (.dbk), JATS journal articles (.nxml), FictionBook e-books (.fb2) and OPML outlines (.opml). DocBook and JATS files saved as plain .xml are recognized too. FictionBook has no lists or code blocks, so on the way out those become paragraphs, and an OPML file opens as a nested list. The AI assistant can convert to them as well.

### Fixed
- Holding Cmd while a flick-scroll coasts into the edge of the document no longer starts zooming.
- Hovering over a toolbar button no longer makes its outline flicker.

## 0.4.0 - 2026-10-01

### Added
- Spell checking now works in German, Spanish, French, Italian, Portuguese and Russian as well as English. Each paragraph is checked in the language it is marked as, or else the one its words are in; text in Chinese, Japanese, Korean or another language without a dictionary isn't checked. Grammar checking is still English only.
- The welcome document no longer shows spelling errors under format names such as OpenDocument and LaTeX, or under key names such as Ctrl and Cmd.
- You can now rename a document: its name is shown in the middle of the menu bar, and clicking it lets you type a new one (Enter to keep it, Esc to cancel). File › Rename does the same. On a narrow window the name is shortened with “…”, or left out when there is no room. The AI assistant can rename the document too.
- View › Page Sidebar (Alt+P) pulls out a column of page thumbnails beside the document, which you can keep open while you edit. It opens by itself when the window is wide enough (not on a phone), until you close it. Click a page to go to it. On a PDF in Edit PDF mode you can also drag pages into a new order and delete them (the × on a page, or the Delete key); undo brings a deleted page back. Thumbnails fill in once the document's fonts and pictures have loaded, instead of staying blank.
- The Cloudflare Billing panel (admins) now lists the previous five months too, each expandable to its charges by service.
- Profile pictures: signing in with Google or GitHub now keeps your picture, shown on the account button, beside your name on your mouse pointer when you edit a document together with others, and next to the people a document is shared with. Microsoft and Apple don't provide one, so those show a plain initial where a picture would go. Sign in again to pick yours up.
- When other people are editing the same document, their pictures now appear beside the document's name in the menu bar (a plain initial for anyone without one, and “…” when there isn't room for them all). Hover them to see who is there.
- The Golem Docs home page is now available in all 11 languages (for example golemdocs.com/home/de). It opens in your language automatically, and there are language links in its footer.

- View › AI Assistant shows or hides the AI assistant (it used to be under Help). Once you hide it with its × button, it stays hidden until you bring it back.
- The AI assistant can merge table cells too.
- The AI assistant can now do nearly everything you can: change fonts, sizes, colors and highlights, set alignment, spacing, bullets and indents, add links, equations, page breaks, tables and columns, set page size, orientation and margins, work with comments and document properties, edit pictures, change the view, download or print, and convert the document to another format (Word, PDF, Markdown and so on). On a PDF it can read pages, highlight and add notes, edit and add text and shapes, change pages and fill in forms.

### Fixed
- Characters that Calibri, Cambria, Arial, Times New Roman, Courier New and Georgia have but the app did not draw (more Latin, Greek and Cyrillic letters, symbols and arrows) now show. They download only for documents that use them.

## 0.3.1 - 2026-10-01

### Changed
- The assistant no longer has a name: it is now just “AI” everywhere, including Help › AI Assistant.

### Fixed
- Subscribing now unlocks syncing, sharing and the AI assistant. Payment notices from Polar were being refused, so a new subscription was never recorded.
- The AI assistant's picture is now even: the left side of the clip no longer sits farther from the ring than the right.

## 0.3.0 - 2026-10-01

### Changed
- The save indicator (a coloured dot, and when the document was last saved) is now its own button, and signing in, your subscription and signing out moved to a new person icon beside it. Signed out, the indicator explains that documents are saved in this browser on this computer only. The offer of a free first month is gone.
- Comments in OpenDocument files now stay on the right text when you edit around them, and show in the comments panel where you can edit them.
- Syncing documents to your account, sharing them and editing together now need a subscription. Golem Docs works fully without one, saving your documents in this browser, and you can still open documents that a subscriber shares with you. If a subscription ends, your documents stay in this browser.
- The website and the welcome document now say clearly that syncing, sharing and the AI assistant need a subscription, and list the price.
- The Terms of Service and Privacy Policy now cover the subscription: what it includes and its limits, that Polar sells and bills it, how the AI assistant handles your text, and what happens when you cancel.

### Added
- Comments now have replies and a Resolve button. Each comment sits beside its text, the open one shows its whole thread and a box to reply in, and there are Previous and Next buttons and a Show resolved switch. Writing or changing a comment happens in the panel instead of a pop-up box.
- Comments are kept in more formats: OpenDocument (.odt), RTF and PDF (as notes) can now be opened and saved with their comments, old Word (.doc) files show their comments when opened, and Markdown (as CriticMarkup), HTML, EPUB and web archives keep them too. Comments carry over when you convert between formats.
- File › Subscribe… takes you to checkout to pay for sync, signing you in first if you are not. Once you subscribe, File › Manage Subscription… opens the page where you change your card, see invoices or cancel. The same buttons are in the panel behind the Saved button at the top right.

### Fixed
- Changing someone's access to a shared document (viewer or editor) now takes effect right away, even if they have it open.
- A new subscription is no longer ended by news that an older one of yours has ended.
- Capitalizing each word with Change Case now works on the words after an emoji.
- In large text files with Windows line endings, a regular-expression search ending in \`$\` now finds the lines it should.
- A large text file opened after searching another no longer shows the old search's highlights.
- The AI assistant no longer fails partway through a request that reads a lot of a long document, and after saying it is unavailable, it offers the chat box again once it is back.
- Editing together recovers from a patchy connection instead of going offline for good, and on a Wi-Fi network that asks you to log in first you stay signed in.
- Opening a file that can't be read now says so instead of leaving the loading screen up, and new documents are named in your language.
- LaTeX, AsciiDoc, Org, Textile and MediaWiki files keep more of their text when opened and saved: LaTeX comment lines no longer split paragraphs and double hyphens stay hyphens; line breaks in list items and numbered lists that start past 1 are kept; tables without a header row stay without one; and text that only looks like markup (plus signs, "Mr." in a table cell, a line like ":note:") is no longer changed or lost.
- CSV files with an unclosed quote, pictures in web pages given as plain (not base64) data, and emails with damaged attachments now open, and text that starts with "BM" is no longer taken for a bitmap picture.
- PDFs whose drawings repeat themselves over and over (or are built to) no longer freeze the page when opened.
- Redacting PDF text shown in a damaged or missing font now takes that text out of the file instead of only covering it. Since which of its letters are under the black box can't be told, the whole line of that text is removed.
- Password-protected PDFs that keep their contents in compressed object streams, and PDFs with a very large number of objects, now open; damaged or hostile PDFs that expand to huge sizes or nest too deeply are read as far as they go instead of freezing or crashing the page.
- Pasting text and a table into the middle of a line no longer leaves an empty paragraph after the table, and deleting table columns next to merged cells keeps those cells the right width.
- Japanese and Chinese RTF files now show their text instead of garbled characters. Web pages, Markdown, RTF and old Word (.doc) files built to be slow or huge (unfinished tags, unclosed comment marks, deep nesting, damaged sector tables) now open quickly or are refused instead of freezing the page. In Markdown tables, links and pictures whose address has a "|" no longer break the table, and email attachments with a quote in their name keep their whole name.
- OpenDocument (.odt) files with tables that repeat rows and columns huge numbers of times, or deeply nested comments, now open instead of freezing the page; languages written with a script (such as Serbian in Latin letters) keep it when saved as .odt; and .odt files are saved valid even when two paragraphs share an id.
- Word documents saved with pasted control characters, or with new properties added to a file from another app, now open in Word; tabs and line breaks in an edited comment are kept; and damaged or oversized .docx files are refused instead of freezing the page.
- Typing in a comment box, a form field of several lines or the Share and Sign In dialogs no longer also acts on the document behind (Backspace deleting its text, Ctrl+Z undoing it, Enter or Esc finishing a crop or drawing); menus no longer open behind a dialog with Alt or F10; a comment with a damaged date no longer says "Invalid Date"; and the email, large-text and picture format names are translated.
- Deleting the open document from the dashboard no longer brings it back at the next autosave, and a half-written comment no longer carries over into the next document you open.
- With formatting marks shown, the dots for spaces now sit on the right characters next to hidden text and page number fields; changing a table style now updates its tables right away; bold italic Greek symbol variants (such as ϵ and ϖ) in equations are drawn bold italic; and undoing a new table row or cell that starts with a nested table removes the right one.

## 0.2.0 - 2026-09-30

### Added
- Clip, a big black binder clip with a face in the bottom right corner, is an assistant for subscribers. Ask it in the speech bubble to read your document, rewrite or add paragraphs, replace text, change styles, format text or leave comments, and Undo takes its changes back. Help › Clip Assistant shows or hides it.
- A command palette button at the top right lists every command, with the ones you used recently first, and lets you search and run them. Press Ctrl+Shift+P (⌘⇧P on Mac) to open it.
- The welcome document now links to the Golem Docs website.

### Changed
- The first-visit welcome box is gone; the Golem Docs website now introduces the app.

### Fixed
- Bulleted and numbered lists in PDFs now keep the PDF's own spacing between the marker and the text, and how far each item is indented, instead of fixed steps.
- Editing a PDF line that starts with a bullet or number no longer squeezes the space between the marker and the text.
- Fixed the welcome page showing a gray box instead of the Golem Docs icon in Safari, including on iPhone and iPad.
- Flicking the page with one finger on a touch screen now keeps scrolling and slows to a stop, instead of halting the moment you lift your finger.
- The can edit / can view dropdown in the Share dialog now looks like the other dropdown menus in the app.

## 0.1.0 - 2026-09-30

### Added
- golemdocs.com/home is a new home page that shows what Golem Docs does, with pages that explain converting formats, editing PDFs and pictures, Word documents, working together, themes, and large text files.
- The plain Privacy Policy and Terms of Service pages are now at golemdocs.com/home/privacy and golemdocs.com/home/terms, and the old addresses redirect there.
- Help › Golem Docs Website opens the new home page.
- Help › What’s New shows this changelog.
- File › Download All (ZIP) downloads every saved document as one ZIP file.
- The home page now says you can download all your saved documents as one ZIP.

### Changed
- File › Convert To now lists formats under the same group headings, in the same order, as the format box.
- On phones and other narrow screens, documents now open in Normal view instead of Print layout, unless you have chosen a view before.
`;export{e as default};
