# Schrumbi user archetypes

Status: **provisional personas**, grounded in published secondary data. Re-derive when primary data exists (see §7).
Evidence grades: **E** = evidence-backed (source cited), **I** = inferred from cited evidence, **A** = assumption (no data yet).

## 1. Method

| Choice | What and why |
|---|---|
| Framework | Cooper's goal-directed personas [S1]: behavioural variables first, place evidence on each variable, cluster by co-occurring behaviour, write goals not tasks. |
| Persona type | Pruitt & Adlin distinguish ad-hoc (assumption) personas from data-driven ones [S2]. Schrumbi has no user data yet, so these are assumption personas whose positions on each variable are anchored in published studies and official statistics. |
| Job framing | Each archetype gets one Jobs-to-be-Done statement ("the progress a person tries to make in a circumstance") [S3], so UI decisions test against outcomes, not features. |
| Quantitative check | Shares come from official statistics (Destatis, Eurostat, DZHW/DSW best3) as in data-driven persona practice [S4]. Salminen et al. warn that data-driven personas lose depth without qualitative grounding [S4]; §7 is the fix. |
| Primary data audit | GitHub on 2026-11-04: repo public since 2026-10-30, 1 star, 0 issues, discussions off, 3 unique visitors in 14 days, 1–4 downloads per release asset. **No external usage evidence exists.** The only observed user is the owner (macOS, German university, Moodle, naming convention like `Muster-M`). The product itself encodes assumptions (Moodle import, per-module submissions, grades, FSRS cards, Obsidian vaults, git) that are treated here as hypotheses, not evidence. |

## 2. Behavioural variables

Scale endpoints used for placement in §4.

| # | Variable | Low end | High end |
|---|---|---|---|
| V1 | Digital literacy | Opens files, uses defaults | Edits config, uses git, scripts |
| V2 | Prior tools | Paper, Word, Moodle downloads folder | Obsidian, Anki, LaTeX, git |
| V3 | Study field load | Text, essays, cases | Formulas, proofs, code |
| V4 | Note-taking style | Handwritten / tablet on lecture PDF | Typed Markdown, linked notes |
| V5 | Math/LaTeX need | None | Daily, inline and display |
| V6 | Organisation style | Reactive, deadline-driven | Planned, system-builder |
| V7 | Device / OS | Windows laptop, mixed with tablet | macOS or Linux, keyboard-centric |
| V8 | Motivation | Extrinsic: pass, grades, deadlines | Intrinsic: knowledge base, mastery |
| V9 | Time pressure | Full-time student | Job, care duties, high workload |
| V10 | Accessibility / executive function | No constraints | ADHD, anxiety, depression, low vision |
| V11 | Language | German only | English or bilingual |
| V12 | Memorisation volume | Low | Very high (medicine, law, languages) |

## 3. Evidence

| ID | Claim | Grade | Source |
|---|---|---|---|
| F1 | German students (16+) with at least basic overall digital skills: 63.9 % (2021), 62.3 % (2023), 72.4 % (2025). EU: 76.8 / 75.2 / 80.1 %. | E | Eurostat isoc_sk_dskl_i21, ind_type STUD [S5] |
| F2 | German students with **above basic** digital skills: 31.7 % (2021), 28.3 % (2023), 34.0 % (2025). EU 2025: 49.7 %. | E | [S5] |
| F3 | German 16–24-year-olds at least basic: 55.7 % (2023), 62.3 % (2025), below EU 70.0 / 74.6 %. "Digital natives" is not a safe assumption in Germany. | E | [S5] |
| F4 | ICILS 2023: over 40 % of German 8th-graders reach only the two lowest of five competence levels; scores fell vs 2013 and 2018. Future students arrive with weaker skills. | E | [S6] |
| F5 | Field mix WS 2023/24 (2,868,311 students): engineering 26.1 %, mathematics/natural sciences 10.5 %, human medicine/health 7.2 %; remaining ~56 % law, economics, social sciences, humanities, arts and others. | E (percentages computed from table) | Destatis [S7] |
| F6 | LaTeX share of published papers: ~92 % mathematics/statistics, ~60 % physics/astronomy, ~45 % computer science. Theses at Masaryk Univ.: ~60 % TeX in informatics, ~15 % in natural sciences and falling. | E (papers, not students) / I for students | Authorea via ASEE [S8]; Brno thesis study [S9] |
| F7 | Word beats LaTeX on speed and errors for plain text; LaTeX wins only for equations; LaTeX users enjoy their tool more. | E (contested design) | Knauff & Nejasmic 2014 [S10] |
| F8 | Medical students in Münster: handwritten notes 51–56 %, tablet notes in the lecture script 35–46 %, laptop notes 2–3 %. | E (one faculty) | Uni Münster MediBib [S11] |
| F9 | Longhand advantage (Mueller & Oppenheimer 2014) did not replicate: no significant difference in Morehead et al. 2019 and Urry et al. 2021 (incl. mini meta-analysis of 8 studies). Medium is a preference, not a learning lever. | E | [S12][S13][S14] |
| F10 | 86 % of students use a laptop or desktop as primary device. | E (US) | EDUCAUSE 2022 [S15] |
| F11 | German desktop OS share Aug 2026: Windows 69.6 %, macOS 21.5 % (OS X + macOS), Linux 7.7 %. Student-specific split unknown. | E (general population) | StatCounter [S16] |
| F12 | ~10,000 German students rate reliable functioning, clear functions and protection of personal data as near-universal requirements for digital tools. | E | HFD Arbeitspapier 54 [S17] |
| F13 | 16 % of students in Germany report a study-impairing condition (2021, up from 11 % in 2016); 65 % of them name a mental illness; 66 % of those report a (very) strong impact on study. | E | best3, DSW/DZHW [S18] |
| F14 | Persistent adult ADHD prevalence 2.58 %, symptomatic 6.76 %. | E | Song et al. 2021 [S19] |
| F15 | 80–95 % of college students procrastinate (figure cited by Steel from earlier work); procrastination is the norm, not a subgroup. | E (secondary estimate) | Steel 2007 [S20] |
| F16 | All students, especially low performers, schedule study by impending deadlines. | E | Hartwig & Dunlosky 2012 [S21] |
| F17 | 84 % of students reread as a strategy; only 20 % were taught how to study. | E | Kornell & Bjork 2007, reported in [S21][S22] |
| F18 | Practice testing and distributed practice are the only two "high utility" techniques; rereading and highlighting are low utility. Spacing effect is large across 317 experiments. | E | Dunlosky et al. 2013 [S23]; Cepeda et al. 2006 [S24] |
| F19 | Anki use among medical students ranges 25 % (Jordan, n=318) to 56 % daily (US cohort) to 80 % (graduate-entry cohort). | E (single sites) | [S25][S26] |
| F20 | Students with ADHD found no organisation tool consistently useful; each "added another layer of burdensome effort". | E (qualitative, n=14) | Syracuse dissertation 2020 [S27] |
| F21 | Broken streaks reduce continued use; highlighting the break makes it worse. | E | Silverman et al. 2023, JCR [S28] |
| F22 | Self-regulated learning is a cycle of forethought (plan, set goals), performance (monitor) and self-reflection (judge against standards). | E (theory) | Zimmerman 2002 [S29] |
| F23 | Users start working instead of learning the tool (production bias) and map new tools onto old ones (assimilation bias). | E (theory, replicated observation) | Carroll & Rosson 1987 [S30] |
| F24 | Students are a minority among PKM-tool adopters: 14 % of Dendron users; Obsidian at 7→10 % of students in one BASB cohort. | E (weak, vendor data) | [S31] |
| F25 | Schrumbi users run it on Windows and Linux in meaningful numbers. | A | Downloads too few to tell |
| F26 | Many target users get submission deadlines from Moodle or ILIAS. | I | HFD questionnaire lists Moodle/ILIAS as core platforms [S17] |

## 4. Variable placement and clusters

L = low end, M = middle, H = high end (§2). Clustering by co-occurring behaviour across V1–V12.

| Var | A Deadline Juggler | B Formula Writer | C Card Crammer | D Restarter | E Vault Veteran |
|---|---|---|---|---|---|
| V1 literacy | L–M (F1, F2) | M–H | M | L–M | H (F2 caps at ~30 %) |
| V2 prior tools | Word, OneNote, GoodNotes, Moodle | LaTeX/Overleaf, Markdown, Jupyter | Anki, AMBOSS-style decks, PDFs | Many abandoned apps (F20) | Obsidian, git, Anki |
| V3 field | Law, econ, social, humanities (F5) | Engineering, maths, CS, physics (F5, F6) | Medicine, law, languages (F5, F19) | Any | Any, often CS |
| V4 notes | Handwritten / tablet on PDF (F8) | Typed Markdown + math | Lecture PDF + cards | Fragmented | Typed, linked |
| V5 math | L | H (F6) | L–M | any | M |
| V6 organisation | Reactive (F16) | Planned for notes, reactive for deadlines | Planned before exams | Reactive, bursts (F15) | System-builder |
| V7 device | Windows likely (F11) | Any, Linux over-represented (A) | Any + tablet | Any | macOS/Linux, keyboard |
| V8 motivation | Extrinsic: pass, on time | Mastery + grades | Exam score | Avoid failure, relief | Intrinsic, tooling |
| V9 time pressure | H (job, care) (A, Sozialerhebung pending) | M | H before exams | H felt | M |
| V10 a11y / EF | Normal | Normal | Normal | H (F13, F14) | Normal |
| V11 language | German | German / English | German | German | English common |
| V12 memorisation | L–M | L | H | M | M |

Shares are each person's **dominant** pattern; traits overlap (every archetype procrastinates sometimes, F15/F16).

## 5. Archetypes

### A. Deadline Juggler (Fristen-Jongleur), primary

Essence: "Tell me what is due, where the file is, and that it got uploaded."

| Field | Content |
|---|---|
| Share | **~40–45 %** (I). 28–38 % of German students lack basic digital skills (F1) and ~70 % lack above-basic (F2); ~56 % study non-formula fields (F5). This is the default student, not a niche. |
| JTBD | When the semester fills with weekly sheets and portal deadlines, I want to see what is due and file each document once, so I never miss a submission. |
| Goals | No missed deadline; find the file for module X; know where they stand on grades. |
| Context | Windows laptop likely (F11), lecture PDFs from Moodle (F26), notes by hand or on a tablet (F8), job beside study (A). |
| Skills | Files and folders yes; Markdown, shortcuts, git no (F2). |
| Comes from | Downloads folder, Moodle, Word, OneNote, GoodNotes, phone calendar. |
| Delights | Moodle calendar fills the overview by itself; drop file → named and filed; "Hochgeladen" tick; everything stays a normal folder. |
| Quits when | First screen asks for setup they do not understand; a note opens as Markdown source; a file seems moved or lost; a German label is jargon ("Vault", "Commit"). |
| Must find without help | Overview, Inbox filing, deadlines, open/annotate PDF, undo. |
| Keep hidden until wanted | Markdown syntax, live mode, graph, git, properties/queries, shortcut config, naming-convention internals. |
| Power expected | Drag-and-drop, right-click, search box. Not keyboard chords. |

Acceptance questions:
1. Can someone who has never seen Markdown finish this flow with mouse only, on Windows, in German?
2. Does every new term on screen exist in everyday German, or is it explained in place?
3. If they click the wrong thing, is the result visible and reversible with one undo, without touching their files?

### B. Formula Writer (Formel-Schreiber), primary

Essence: "My notes are maths. Typing them must be as fast as writing them."

| Field | Content |
|---|---|
| Share | **~20 %** (I). Engineering + maths/natural sciences = 36.6 % (F5); LaTeX dominance in maths/CS/physics publishing (F6) suggests roughly half of those write formulas regularly. |
| JTBD | When I work through proofs and exercise sheets, I want to type math inline next to the lecture PDF, so my notes are searchable and exportable as a clean PDF. |
| Goals | Fast, correct math rendering; split view PDF + note; export to PDF for submission; weekly sheet tracking. |
| Context | Typed notes, often bilingual material, may use Linux (A), knows some LaTeX (F6, F7). |
| Skills | Medium–high; learns syntax willingly if it pays off; keyboard-first for typing. |
| Comes from | Overleaf, LaTeX, Jupyter, Obsidian/Typora, GoodNotes for sketches. |
| Delights | `$…$` renders instantly; LaTeX macros they know just work; PDF export looks typeset; side-by-side PDF. |
| Quits when | Math renders wrong or slowly; cursor jumps while typing; export loses formulas; editor lags (frame budget). |
| Must find without help | New note on a submission, math preview, split view, PDF export. |
| Keep hidden until wanted | Card scheduling, grades detail, git. |
| Power expected | Shortcuts for editing, snippets/completion, palette, find in note. |

Acceptance questions:
1. Does typing math stay within the frame budget with a long note and many formulas?
2. Do common LaTeX constructs (aligned, matrices, `\mathbb`, macros) render and export identically?
3. Can they go from exercise PDF to a submitted PDF without leaving Schrumbi or the keyboard?

### C. Card Crammer (Karten-Lerner), primary

Essence: "Thousands of facts, one exam date, every day counts."

| Field | Content |
|---|---|
| Share | **~10–15 %** (I). Medicine/health 7.2 % (F5) plus law and languages; Anki use 25–80 % in medicine (F19). |
| JTBD | When an exam is weeks away, I want my daily reviews ready from my own notes and imported decks, so I remember enough on exam day. |
| Goals | Daily review done fast; import existing Anki decks unchanged; see readiness per exam. |
| Context | High volume, short sessions, exam-driven (F16); notes on lecture PDFs (F8). |
| Skills | Medium; knows Anki deeply, not Markdown. |
| Comes from | Anki, shared community decks, PDF scripts. |
| Delights | Anki import keeps progress and media; cards live next to the note; "Vor der Prüfung" mode; spacing is evidence-based (F18). |
| Quits when | Import loses cards or scheduling; reviewing is slower than Anki; a missed day turns into guilt (F21). |
| Must find without help | Karten, import, today's review, exam date. |
| Keep hidden until wanted | FSRS parameters, card syntax variants. |
| Power expected | One-key grading, fast review loop, bulk import. |

Acceptance questions:
1. Is a 300-card review session as fast per card as Anki, keyboard only?
2. Does an imported deck round-trip without losing cards, media or due dates?
3. After a week away, does the app show a doable plan instead of a wall of overdue cards?

### D. Restarter (Neustarter), primary, cross-cutting

Essence: "I start late, lose track, and need the next small step, not another system."

| Field | Content |
|---|---|
| Share | **~15 %** as dominant pattern (I). 16 % report a study-impairing condition, two-thirds mental illness (F13); symptomatic adult ADHD 6.8 % (F14); procrastination near-universal (F15, F16). Their needs apply to everyone under stress. |
| JTBD | When I fall behind and feel overwhelmed, I want one clear next action and proof I am catching up, so I can restart without shame. |
| Goals | Know the single most urgent thing; start it in one click; see progress. |
| Context | Bursty work, many abandoned tools (F20), high anxiety around overdue items. |
| Skills | Any; low tolerance for setup and maintenance. |
| Comes from | Paper lists, phone reminders, abandoned Notion/Todoist setups. |
| Delights | "Jetzt dran" card with one button; low setup; quick wins visible; nothing nags. |
| Quits when | Setup takes more than a few minutes; red overdue walls; broken streak highlighted (F21); the tool needs upkeep (F20). |
| Must find without help | Overview "Jetzt dran", status circle, undo. |
| Keep hidden until wanted | Everything optional: streak counters, grades, statistics, tags, graph. |
| Power expected | None required; respond well to sane defaults. |

Acceptance questions:
1. Does the screen show one obvious next action within a second of opening?
2. Does a missed day or broken streak avoid blame, red walls and loss framing?
3. Does this feature work with zero configuration and need no upkeep?

### E. Vault Veteran (Vault-Profi), secondary

Essence: "I already have a system; plug into it, do not replace it."

| Field | Content |
|---|---|
| Share | **~5–10 %** (I). Above-basic digital skills cap at ~30 % of German students (F2); students are a minority of PKM-tool users (F24). Loud and visible, so easy to over-weight. |
| JTBD | When I add study planning to my existing vault, I want Schrumbi to read and write my files without breaking Obsidian or git, so I keep one source of truth. |
| Goals | Open existing vault; links, embeds, properties compatible; git sync; keyboard everything. |
| Context | macOS or Linux, keyboard-centric, English UI acceptable (A). |
| Skills | High: Markdown, git, config files. |
| Comes from | Obsidian (+ plugins), VS Code, Anki, git. |
| Delights | Zero-diff on their vault; palette with every command; configurable shortcuts; fast at 20k notes. |
| Quits when | Schrumbi rewrites files, adds clutter, or breaks Obsidian syntax; no keyboard path; slow on big vaults. |
| Must find without help | Open vault, palette, shortcut overview, sync. |
| Keep hidden until wanted | Onboarding tips, gamification. |
| Power expected | Palette, rebindable keys, queries, graph, git conflict handling. |

Acceptance questions:
1. Does the change leave every file Schrumbi did not need to touch byte-identical, and stay readable by Obsidian?
2. Is there a keyboard path and a palette entry for it?
3. Does it hold the frame budget at 20,000 notes?

### Anti-persona: Workspace Builder

Wants an everything-app: real-time team editing, databases, plugins/scripting, mobile-first, cloud accounts (Notion, Google Docs). Justified because Schrumbi's core promises (local plain folders, desktop, opinionated study model; F12 values data protection) cannot serve this without adding complexity that drives A and D away (F20, F23). Requests from this profile (plugin API, shared workspaces, web clipper) are declined by default.

## 6. Design principles for beginner/expert tension

| Principle | Rule for Schrumbi | Serves | Evidence |
|---|---|---|---|
| Progressive disclosure | First screen = Overview, Inbox, PDFs. Markdown source, graph, queries, git and statistics appear only after first use or opt-in. Never remove the expert path, only defer it. | A, D first; B, E keep power | Nielsen [S32]; F2, F23 |
| Sensible defaults, zero-config core | Every feature must work with no settings touched; setup steps are skippable; settings refine, never enable the basics. | A, D | F20, F23, F12 |
| Safe exploration | Every action on user files is undoable in one step and visible; never delete or rewrite silently; show what changed. | All, A most | Shneiderman rule "easy reversal" [S33]; F12 |
| Two paths to every command | Mouse/right-click path for A/C/D, palette and rebindable shortcut for B/E. Shortcuts shown in menus and tooltips so users graduate. | B, E without costing A | Shneiderman "shortcuts for frequent users" [S33]; F23 |
| Plain language | Everyday German in UI; no "Vault", "Commit", "Frontmatter" on default paths; tool terms only inside expert features. | A, D | F1–F4 |
| Gentle accountability | Progress shown as what got done, not what was missed; no loss framing on streaks; overdue items grouped and actionable. | D, C | F21, F13, F15 |
| Evidence-based learning by default | Cards use spacing (FSRS) and active recall; offer "Karte aus Notiz" over highlighting tools. | C, B | F17, F18 |
| Support the SRL cycle | Forethought = semester/deadline planning, performance = notes/cards/progress, reflection = grades and weekly progress. Each phase reachable in one click from Overview. | All | F22 |
| Respect the folder | Files stay readable outside Schrumbi and by Obsidian; no clutter beyond documented dotfiles. | E, A | F12; product promise |
| Platform parity | Windows is the likely majority desktop (F11); German UI quality and Windows behaviour count as much as macOS polish. | A | F11, F25 |
| Accessibility baseline | Scalable text, keyboard reachability, contrast and reduced motion per WCAG 2.2. | D, all | WCAG 2.2 [S34]; F13 |

## 7. Validation plan (to replace assumptions)

1. In-app opt-in, anonymous, one-time survey after 2 weeks: field (Destatis groups), OS, prior tools, note medium, LaTeX use, Anki use, job hours, self-reported study impairment. Map answers onto V1–V12.
2. 5–8 contextual interviews per primary archetype at German universities (A: common practice, size not sourced).
3. Recompute shares from survey; drop or merge any archetype under 5 %.
4. Re-check F25 with release download counts per OS once downloads exceed ~200.
5. Pending source: DZHW 22. Sozialerhebung tables on working hours (V9) and DZHW Studierendenbefragung 2021 device data; not retrieved here.

## Sources

| ID | Source | Supports |
|---|---|---|
| S1 | Cooper, Reimann, Cronin, Noessel: *About Face*, 4th ed., Wiley 2014; overview https://en.wikipedia.org/wiki/Alan_Cooper | Method, behavioural variables |
| S2 | Pruitt & Adlin: *The Persona Lifecycle*, Morgan Kaufmann 2006. https://shop.elsevier.com/books/the-persona-lifecycle/pruitt/978-0-12-566251-2 | Assumption vs data-driven personas |
| S3 | Christensen, Hall, Dillon, Duncan: Know Your Customers' "Jobs to Be Done", HBR Sept 2016. https://hbr.org/2016/09/know-your-customers-jobs-to-be-done | JTBD |
| S4 | Salminen, Guan, Jung, Jansen: A Survey of 15 Years of Data-Driven Persona Development, IJHCI 37(18), 2021. https://www.utupub.fi/bitstream/10024/172489/1/A%20Survey%20of%2015%20Years%20of%20Data%20Driven%20Persona%20Development.pdf | Data-driven personas, depth risk |
| S5 | Eurostat isoc_sk_dskl_i21, Individuals' level of digital skills, queried via API 2026-11-04. https://ec.europa.eu/eurostat/databrowser/view/isoc_sk_dskl_i21/default/table | F1–F3 |
| S6 | ICILS 2023 Germany, Univ. Paderborn press release. https://uni-paderborn.de/nachricht/143045 ; https://idw-online.de/de/news842780 | F4 |
| S7 | Destatis, Studierende nach Fächergruppen. https://www.destatis.de/DE/Themen/Gesellschaft-Umwelt/Bildung-Forschung-Kultur/Hochschulen/Tabellen/studierende-insgesamt-faechergruppe.html | F5 |
| S8 | ASEE paper citing Authorea LaTeX shares. https://peer.asee.org/introducing-latex-to-the-academic-researcher-engineering-writing-with-a-difference-resubmission.pdf | F6 |
| S9 | Zpravodaj CSTUG 2015, TeX use in theses at Masaryk Univ. https://www.dml.cz/bitstream/handle/10338.dmlcz/150229/ZpravodajCSTUG_025-2015-1_4.pdf | F6 |
| S10 | Knauff & Nejasmic: An Efficiency Comparison of Document Preparation Systems, PLOS ONE 2014. https://pmc.ncbi.nlm.nih.gov/articles/PMC4272305 | F7 |
| S11 | Uni Münster MediBib, easyphysikum surveys. https://www.uni-muenster.de/MediBib/aktuelles/5242 ; https://www.uni-muenster.de/MediBib/aktuelles/5305 | F8 |
| S12 | Mueller & Oppenheimer: The Pen Is Mightier Than the Keyboard, Psych. Science 25, 2014 (cited via S13) | F9 |
| S13 | Morehead, Dunlosky, Rawson: How Much Mightier Is the Pen than the Keyboard?, Educ. Psych. Review 31, 2019. https://doi.org/10.1007/s10648-019-09468-2 | F9 |
| S14 | Urry et al.: Don't Ditch the Laptop Just Yet, Psych. Science 32(3), 2021. https://journals.sagepub.com/doi/10.1177/0956797620965541 | F9 |
| S15 | EDUCAUSE Students and Technology 2022 summary. https://uhclblackboardtips.blogspot.com/2022/10/educause-2022-students-and-technology.html ; https://er.educause.edu/articles/2023/1/the-evolving-landscape-of-students-mobile-learning-practices-in-higher-education | F10 |
| S16 | StatCounter desktop OS share Germany. https://gs.statcounter.com/os-market-share/desktop/germany | F11 |
| S17 | Hochschulforum Digitalisierung, Arbeitspapier 54, Studierendenbefragung, 2020. https://zenodo.org/records/4275913 | F12, F26 |
| S18 | best3 summary, Deutsches Studierendenwerk. https://www.studierendenwerke.de/fileadmin/api/files/Fotos_Studienfinanzierung/best3_summary_DE_bf.pdf ; https://www.studierendenwerke.de/beitrag/viel-mehr-studierende-mit-psychischen-erkrankungen | F13 |
| S19 | Song et al.: The prevalence of adult ADHD, J. Global Health 11, 2021. https://www.jogh.org/documents/2021/jogh-11-04009.htm | F14 |
| S20 | Steel: The Nature of Procrastination, Psych. Bulletin 133, 2007. https://prism.ucalgary.ca/handle/1880/47914 | F15 |
| S21 | Hartwig & Dunlosky: Study strategies of college students, Psychon. Bull. Rev. 19, 2012. https://dx.doi.org/10.3758/s13423-011-0181-y | F16, F17 |
| S22 | Learning Scientists: Do Students Utilize Effective Learning Strategies? https://www.learningscientists.org/blog/2018/2/8-1 | F17 |
| S23 | Dunlosky et al.: Improving Students' Learning With Effective Learning Techniques, PSPI 2013. https://www.psychologicalscience.org/news/releases/which-study-strategies-make-the-grade.html | F18 |
| S24 | Cepeda et al.: Distributed practice in verbal recall tasks, Psych. Bulletin 132, 2006. https://pubmed.ncbi.nlm.nih.gov/16719566/ | F18 |
| S25 | Utilization Patterns of Anki among first-year medical students, Cureus 2025. https://www.ncbi.nlm.nih.gov/pmc/articles/PMC12662189/ | F19 |
| S26 | Academic and Wellness Outcomes of Anki use in medical school, JMECD 2023. https://doaj.org/article/7aaf357db5d54ad39fea3f672ad5aae4 ; Jordan cohort https://pmc.ncbi.nlm.nih.gov/articles/PMC10176558 | F19 |
| S27 | Syracuse University dissertation on community-college students with ADHD, 2020. https://surface.syr.edu/etd/1172 | F20 |
| S28 | Silverman, Barasch, Inman, Wood, Lee: On or Off Track: How (Broken) Streaks Affect Consumer Decisions, JCR 49(6), 2023. https://doi.org/10.1093/jcr/ucac029 | F21 |
| S29 | Zimmerman: Becoming a Self-Regulated Learner, Theory Into Practice 41(2), 2002. https://doi.org/10.1207/s15430421tip4102_2 | F22 |
| S30 | Carroll & Rosson: Paradox of the Active User, in *Interfacing Thought*, MIT Press 1987; NN/g summary https://www.nngroup.com/articles/paradox-of-the-active-user/ | F23 |
| S31 | Dendron new user survey 2021. https://dev.to/dendron/2021-dendron-new-user-survey-results-1736 | F24 |
| S32 | Nielsen: Progressive Disclosure, NN/g 2006. https://www.nngroup.com/articles/progressive-disclosure/ | §6 |
| S33 | Shneiderman: The Eight Golden Rules of Interface Design. https://www.cs.umd.edu/users/ben/goldenrules.html | §6 |
| S34 | W3C WCAG 2.2. https://www.w3.org/TR/WCAG22/ | §6 |
