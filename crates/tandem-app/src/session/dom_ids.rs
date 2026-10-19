//! The DOM-element constants of renderer.ts and redact-ui.ts, mapped to the element ids of
//! `src/renderer/index.html` (the keys of [`crate::dom::Dom`]), and [`init_dom`]: the initial
//! state index.html gives each element (text, value, hidden, disabled, open, checked and the
//! aria-*/data-*/title attributes the logic reads back).
//!
//! Generated from the TypeScript and index.html; `R:n` is the renderer.ts line, `U:n` redact-ui.ts.
//! A TS name `statusEl` becomes `STATUS_EL`. The macOS wording of index.html is kept (the
//! renderer's `platformWording` rewrite applies to other platforms only).

use super::types::UiMode;
use crate::dom::{Dom, Node};

/// `statusEl` (R:253).
pub const STATUS_EL: &str = "status";
/// `errorEl` (R:254).
pub const ERROR_EL: &str = "error";
/// `bannerEl` (R:255).
pub const BANNER_EL: &str = "banner";
/// `bannerTextEl` (R:256).
pub const BANNER_TEXT_EL: &str = "banner-text";
/// `bannerDismissEl` (R:257).
pub const BANNER_DISMISS_EL: &str = "banner-dismiss";
/// `warningEl` (R:258).
pub const WARNING_EL: &str = "warning";
/// `canvas` (R:259).
pub const CANVAS: &str = "canvas";
/// `overlay` (R:260).
pub const OVERLAY: &str = "text-layer";
/// `hitLayer` (R:261).
pub const HIT_LAYER: &str = "hit-layer";
/// `fieldBar` (R:262).
pub const FIELD_BAR: &str = "field-bar";
/// `fieldCount` (R:263).
pub const FIELD_COUNT: &str = "field-count";
/// `fieldHighlight` (R:264).
pub const FIELD_HIGHLIGHT: &str = "field-highlight";
/// `fieldNext` (R:266).
pub const FIELD_NEXT: &str = "field-next";
/// `extracted` (R:267).
pub const EXTRACTED: &str = "extracted";
/// `caption` (R:268).
pub const CAPTION: &str = "caption";
/// `viewport` (R:269).
pub const VIEWPORT: &str = "viewport";
/// `pageRegion` (R:270).
pub const PAGE_REGION: &str = "page";
/// `objectMenu` (R:272).
pub const OBJECT_MENU: &str = "object-menu";
/// `pageInput` (R:300).
pub const PAGE_INPUT: &str = "page-input";
/// `prevButton` (R:301).
pub const PREV_BUTTON: &str = "prev";
/// `nextButton` (R:302).
pub const NEXT_BUTTON: &str = "next";
/// `openButton` (R:303).
pub const OPEN_BUTTON: &str = "open";
/// `saveButton` (R:304).
pub const SAVE_BUTTON: &str = "save";
/// `saveCopyButton` (R:305).
pub const SAVE_COPY_BUTTON: &str = "save-copy";
/// `undoButton` (R:306).
pub const UNDO_BUTTON: &str = "undo";
/// `redoButton` (R:307).
pub const REDO_BUTTON: &str = "redo";
/// `editStatusEl` (R:308).
pub const EDIT_STATUS_EL: &str = "edit-status";
/// `notesHeading` (R:309).
pub const NOTES_HEADING: &str = "notes-heading";
/// `notesEmptyEl` (R:310).
pub const NOTES_EMPTY_EL: &str = "notes-empty";
/// `noteAddButton` (R:311).
pub const NOTE_ADD_BUTTON: &str = "note-add";
/// `markupAllButton` (R:318).
pub const MARKUP_ALL_BUTTON: &str = "markup-highlight-all";
/// `freeTextAddButton` (R:319).
pub const FREE_TEXT_ADD_BUTTON: &str = "freetext-add";
/// `inkDrawButton` (R:325).
pub const INK_DRAW_BUTTON: &str = "ink-draw";
/// `stampIconButton` (R:326).
pub const STAMP_ICON_BUTTON: &str = "stamp-icon";
/// `signButton` (R:327).
pub const SIGN_BUTTON: &str = "sign-add";
/// `stampIconDialog` (R:328).
pub const STAMP_ICON_DIALOG: &str = "stamp-icon-dialog";
/// `stampIconSaveButton` (R:329).
pub const STAMP_ICON_SAVE_BUTTON: &str = "stamp-icon-save";
/// `stampIconCancelButton` (R:330).
pub const STAMP_ICON_CANCEL_BUTTON: &str = "stamp-icon-cancel";
/// `noteListEl` (R:331).
pub const NOTE_LIST_EL: &str = "note-list";
/// `noteDialog` (R:332).
pub const NOTE_DIALOG: &str = "note-dialog";
/// `noteDialogHeading` (R:333).
pub const NOTE_DIALOG_HEADING: &str = "note-dialog-heading";
/// `noteTextInput` (R:334).
pub const NOTE_TEXT_INPUT: &str = "note-text";
/// `noteSaveButton` (R:335).
pub const NOTE_SAVE_BUTTON: &str = "note-save";
/// `noteCancelButton` (R:336).
pub const NOTE_CANCEL_BUTTON: &str = "note-cancel";
/// `zoomOutButton` (R:337).
pub const ZOOM_OUT_BUTTON: &str = "zoom-out";
/// `zoomInButton` (R:338).
pub const ZOOM_IN_BUTTON: &str = "zoom-in";
/// `fitWidthButton` (R:339).
pub const FIT_WIDTH_BUTTON: &str = "fit-width";
/// `fitPageButton` (R:340).
pub const FIT_PAGE_BUTTON: &str = "fit-page";
/// `zoom100Button` (R:341).
pub const ZOOM100_BUTTON: &str = "zoom-100";
/// `zoomLevel` (R:342).
pub const ZOOM_LEVEL: &str = "zoom-level";
/// `searchInput` (R:343).
pub const SEARCH_INPUT: &str = "search-input";
/// `searchPrevButton` (R:344).
pub const SEARCH_PREV_BUTTON: &str = "search-prev";
/// `searchNextButton` (R:345).
pub const SEARCH_NEXT_BUTTON: &str = "search-next";
/// `searchStatusEl` (R:346).
pub const SEARCH_STATUS_EL: &str = "search-status";
/// `thumbsEl` (R:347).
pub const THUMBS_EL: &str = "thumbs";
/// `thumbsToggle` (R:348).
pub const THUMBS_TOGGLE: &str = "thumbs-toggle";
/// `sidebarEl` (R:349).
pub const SIDEBAR_EL: &str = "page-sidebar";
/// `dropHint` (R:350).
pub const DROP_HINT: &str = "drop-hint";
/// `passwordForm` (R:351).
pub const PASSWORD_FORM: &str = "password-form";
/// `passwordName` (R:352).
pub const PASSWORD_NAME: &str = "password-name";
/// `passwordInput` (R:353).
pub const PASSWORD_INPUT: &str = "password-input";
/// `passwordErrorEl` (R:354).
pub const PASSWORD_ERROR_EL: &str = "password-error";
/// `printButton` (R:355).
pub const PRINT_BUTTON: &str = "print";
/// `printDialog` (R:356).
pub const PRINT_DIALOG: &str = "print-dialog";
/// `printRangeInput` (R:357).
pub const PRINT_RANGE_INPUT: &str = "print-range";
/// `printStatusEl` (R:358).
pub const PRINT_STATUS_EL: &str = "print-status";
/// `printStartButton` (R:359).
pub const PRINT_START_BUTTON: &str = "print-start";
/// `printCancelButton` (R:360).
pub const PRINT_CANCEL_BUTTON: &str = "print-cancel";
/// `printPagesEl` (R:361).
pub const PRINT_PAGES_EL: &str = "print-pages";
/// `pageStatusEl` (R:362).
pub const PAGE_STATUS_EL: &str = "page-status";
/// `fieldsEl` (R:363).
pub const FIELDS_EL: &str = "fields";
/// `fieldsHeading` (R:364).
pub const FIELDS_HEADING: &str = "fields-heading";
/// `fieldsEmptyEl` (R:365).
pub const FIELDS_EMPTY_EL: &str = "fields-empty";
/// `fillSignButton` (R:366).
pub const FILL_SIGN_BUTTON: &str = "fill-sign";
/// `fillSignDrawButton` (R:367).
pub const FILL_SIGN_DRAW_BUTTON: &str = "fill-sign-draw";
/// `sigpadDialog` (R:368).
pub const SIGPAD_DIALOG: &str = "sigpad-dialog";
/// `sigpadCanvas` (R:369).
pub const SIGPAD_CANVAS: &str = "sigpad-canvas";
/// `sigpadStatus` (R:370).
pub const SIGPAD_STATUS: &str = "sigpad-status";
/// `sigpadClearButton` (R:371).
pub const SIGPAD_CLEAR_BUTTON: &str = "sigpad-clear";
/// `sigpadImageButton` (R:372).
pub const SIGPAD_IMAGE_BUTTON: &str = "sigpad-image";
/// `sigpadRemoveButton` (R:373).
pub const SIGPAD_REMOVE_BUTTON: &str = "sigpad-remove";
/// `sigpadSaveButton` (R:374).
pub const SIGPAD_SAVE_BUTTON: &str = "sigpad-save";
/// `sigpadCancelButton` (R:375).
pub const SIGPAD_CANCEL_BUTTON: &str = "sigpad-cancel";
/// `fieldsStatusEl` (R:376).
pub const FIELDS_STATUS_EL: &str = "fields-status";
/// `fieldsWarningEl` (R:377).
pub const FIELDS_WARNING_EL: &str = "fields-warning";
/// `fieldListEl` (R:378).
pub const FIELD_LIST_EL: &str = "field-list";
/// `designerEl` (R:380).
pub const DESIGNER_EL: &str = "field-designer";
/// `designerHintEl` (R:381).
pub const DESIGNER_HINT_EL: &str = "field-designer-hint";
/// `fieldNewKindEl` (R:382).
pub const FIELD_NEW_KIND_EL: &str = "field-new-kind";
/// `fieldNewNameEl` (R:383).
pub const FIELD_NEW_NAME_EL: &str = "field-new-name";
/// `fieldNewXEl` (R:384).
pub const FIELD_NEW_XEL: &str = "field-new-x";
/// `fieldNewYEl` (R:385).
pub const FIELD_NEW_YEL: &str = "field-new-y";
/// `fieldNewWEl` (R:386).
pub const FIELD_NEW_WEL: &str = "field-new-w";
/// `fieldNewHEl` (R:387).
pub const FIELD_NEW_HEL: &str = "field-new-h";
/// `fieldNewRectEl` (R:388).
pub const FIELD_NEW_RECT_EL: &str = "field-new-rect";
/// `fieldNewTextEl` (R:389).
pub const FIELD_NEW_TEXT_EL: &str = "field-new-text";
/// `fieldNewComboEl` (R:390).
pub const FIELD_NEW_COMBO_EL: &str = "field-new-combo";
/// `fieldNewRadioEl` (R:391).
pub const FIELD_NEW_RADIO_EL: &str = "field-new-radio";
/// `fieldNewMaxLenEl` (R:392).
pub const FIELD_NEW_MAX_LEN_EL: &str = "field-new-maxlen";
/// `fieldNewMultilineEl` (R:393).
pub const FIELD_NEW_MULTILINE_EL: &str = "field-new-multiline";
/// `fieldNewOptionsEl` (R:394).
pub const FIELD_NEW_OPTIONS_EL: &str = "field-new-options";
/// `fieldNewKidsEl` (R:395).
pub const FIELD_NEW_KIDS_EL: &str = "field-new-kids";
/// `fieldNewLabelEl` (R:397).
pub const FIELD_NEW_LABEL_EL: &str = "field-new-label";
/// `fieldNewReadOnlyEl` (R:398).
pub const FIELD_NEW_READ_ONLY_EL: &str = "field-new-readonly";
/// `fieldNewRequiredEl` (R:399).
pub const FIELD_NEW_REQUIRED_EL: &str = "field-new-required";
/// `fieldNewDefaultTextEl` (R:400).
pub const FIELD_NEW_DEFAULT_TEXT_EL: &str = "field-new-default";
/// `fieldNewDefaultBoolEl` (R:401).
pub const FIELD_NEW_DEFAULT_BOOL_EL: &str = "field-new-default-check";
/// `fieldNewAddEl` (R:402).
pub const FIELD_NEW_ADD_EL: &str = "field-new-add";
/// `fieldSelectEl` (R:403).
pub const FIELD_SELECT_EL: &str = "field-select";
/// `fieldPropNameEl` (R:404).
pub const FIELD_PROP_NAME_EL: &str = "field-prop-name";
/// `fieldPropXEl` (R:405).
pub const FIELD_PROP_XEL: &str = "field-prop-x";
/// `fieldPropYEl` (R:406).
pub const FIELD_PROP_YEL: &str = "field-prop-y";
/// `fieldPropWEl` (R:407).
pub const FIELD_PROP_WEL: &str = "field-prop-w";
/// `fieldPropHEl` (R:408).
pub const FIELD_PROP_HEL: &str = "field-prop-h";
/// `fieldPropRectEl` (R:409).
pub const FIELD_PROP_RECT_EL: &str = "field-prop-rect";
/// `fieldPropTextEl` (R:410).
pub const FIELD_PROP_TEXT_EL: &str = "field-prop-text";
/// `fieldPropComboEl` (R:411).
pub const FIELD_PROP_COMBO_EL: &str = "field-prop-combo";
/// `fieldPropMaxLenEl` (R:412).
pub const FIELD_PROP_MAX_LEN_EL: &str = "field-prop-maxlen";
/// `fieldPropMultilineEl` (R:413).
pub const FIELD_PROP_MULTILINE_EL: &str = "field-prop-multiline";
/// `fieldPropOptionsEl` (R:414).
pub const FIELD_PROP_OPTIONS_EL: &str = "field-prop-options";
/// `fieldPropLabelEl` (R:415).
pub const FIELD_PROP_LABEL_EL: &str = "field-prop-label";
/// `fieldPropReadOnlyEl` (R:416).
pub const FIELD_PROP_READ_ONLY_EL: &str = "field-prop-readonly";
/// `fieldPropRequiredEl` (R:417).
pub const FIELD_PROP_REQUIRED_EL: &str = "field-prop-required";
/// `fieldPropDefaultTextEl` (R:418).
pub const FIELD_PROP_DEFAULT_TEXT_EL: &str = "field-prop-default";
/// `fieldPropDefaultBoolEl` (R:419).
pub const FIELD_PROP_DEFAULT_BOOL_EL: &str = "field-prop-default-check";
/// `fieldPropApplyEl` (R:420).
pub const FIELD_PROP_APPLY_EL: &str = "field-prop-apply";
/// `fieldPropDeleteEl` (R:421).
pub const FIELD_PROP_DELETE_EL: &str = "field-prop-delete";
/// `helpDialog` (R:423).
pub const HELP_DIALOG: &str = "help-dialog";
/// `helpMatrixEl` (R:424).
pub const HELP_MATRIX_EL: &str = "help-matrix";
/// `helpCloseEl` (R:425).
pub const HELP_CLOSE_EL: &str = "help-close";
/// `redactAreaButton` (R:426).
pub const REDACT_AREA_BUTTON: &str = "redact-area";
/// `redactDialog` (R:427).
pub const REDACT_DIALOG: &str = "redact-dialog";
/// `outlineListEl` (R:430).
pub const OUTLINE_LIST_EL: &str = "outline-list";
/// `bookmarkAddButton` (R:431).
pub const BOOKMARK_ADD_BUTTON: &str = "bookmark-add";
/// `linksHeading` (R:432).
pub const LINKS_HEADING: &str = "links-heading";
/// `linkListEl` (R:433).
pub const LINK_LIST_EL: &str = "link-list";
/// `linkPageAddButton` (R:434).
pub const LINK_PAGE_ADD_BUTTON: &str = "link-page-add";
/// `linkUriAddButton` (R:435).
pub const LINK_URI_ADD_BUTTON: &str = "link-uri-add";
/// `navStatusEl` (R:436).
pub const NAV_STATUS_EL: &str = "nav-status";
/// `bookmarkDialog` (R:437).
pub const BOOKMARK_DIALOG: &str = "bookmark-dialog";
/// `bookmarkDialogHeading` (R:438).
pub const BOOKMARK_DIALOG_HEADING: &str = "bookmark-dialog-heading";
/// `bookmarkTitleInput` (R:439).
pub const BOOKMARK_TITLE_INPUT: &str = "bookmark-title";
/// `bookmarkSaveButton` (R:440).
pub const BOOKMARK_SAVE_BUTTON: &str = "bookmark-save";
/// `bookmarkCancelButton` (R:441).
pub const BOOKMARK_CANCEL_BUTTON: &str = "bookmark-cancel";
/// `linkDialog` (R:442).
pub const LINK_DIALOG: &str = "link-dialog";
/// `linkDialogHeading` (R:443).
pub const LINK_DIALOG_HEADING: &str = "link-dialog-heading";
/// `linkTargetLabel` (R:444).
pub const LINK_TARGET_LABEL: &str = "link-target-label";
/// `linkTargetInput` (R:445).
pub const LINK_TARGET_INPUT: &str = "link-target";
/// `linkSaveButton` (R:446).
pub const LINK_SAVE_BUTTON: &str = "link-save";
/// `linkCancelButton` (R:447).
pub const LINK_CANCEL_BUTTON: &str = "link-cancel";
/// `linkKindSelect` (R:448).
pub const LINK_KIND_SELECT: &str = "link-kind";
/// `linkDrawButton` (R:449).
pub const LINK_DRAW_BUTTON: &str = "link-draw";
/// `sigfieldDrawButton` (R:450).
pub const SIGFIELD_DRAW_BUTTON: &str = "sigfield-draw";
/// `editOutlinesInput` (R:451).
pub const EDIT_OUTLINES_INPUT: &str = "edit-outlines";
/// `pageLabelEl` (R:460).
pub const PAGE_LABEL_EL: &str = "page-label";
/// `watermarkButton` (R:461).
pub const WATERMARK_BUTTON: &str = "stamp-watermark";
/// `headerButton` (R:462).
pub const HEADER_BUTTON: &str = "stamp-header";
/// `pageLabelsButton` (R:463).
pub const PAGE_LABELS_BUTTON: &str = "page-labels";
/// `stampDialog` (R:464).
pub const STAMP_DIALOG: &str = "stamp-dialog";
/// `stampDialogHeading` (R:465).
pub const STAMP_DIALOG_HEADING: &str = "stamp-dialog-heading";
/// `stampKindSelect` (R:466).
pub const STAMP_KIND_SELECT: &str = "stamp-kind";
/// `stampTextInput` (R:467).
pub const STAMP_TEXT_INPUT: &str = "stamp-text";
/// `stampPagesInput` (R:468).
pub const STAMP_PAGES_INPUT: &str = "stamp-pages";
/// `stampAlignSelect` (R:469).
pub const STAMP_ALIGN_SELECT: &str = "stamp-align";
/// `stampBatesStartInput` (R:470).
pub const STAMP_BATES_START_INPUT: &str = "stamp-bates-start";
/// `stampBatesPadInput` (R:471).
pub const STAMP_BATES_PAD_INPUT: &str = "stamp-bates-pad";
/// `stampSaveButton` (R:472).
pub const STAMP_SAVE_BUTTON: &str = "stamp-save";
/// `stampCancelButton` (R:473).
pub const STAMP_CANCEL_BUTTON: &str = "stamp-cancel";
/// `labelsDialog` (R:474).
pub const LABELS_DIALOG: &str = "labels-dialog";
/// `labelsFromInput` (R:475).
pub const LABELS_FROM_INPUT: &str = "labels-from";
/// `labelsStyleSelect` (R:476).
pub const LABELS_STYLE_SELECT: &str = "labels-style";
/// `labelsPrefixInput` (R:477).
pub const LABELS_PREFIX_INPUT: &str = "labels-prefix";
/// `labelsStartInput` (R:478).
pub const LABELS_START_INPUT: &str = "labels-start";
/// `labelsSaveButton` (R:479).
pub const LABELS_SAVE_BUTTON: &str = "labels-save";
/// `labelsClearButton` (R:480).
pub const LABELS_CLEAR_BUTTON: &str = "labels-clear";
/// `labelsCancelButton` (R:481).
pub const LABELS_CANCEL_BUTTON: &str = "labels-cancel";
/// `layersEl` (R:482).
pub const LAYERS_EL: &str = "layers";
/// `layerListEl` (R:483).
pub const LAYER_LIST_EL: &str = "layer-list";
/// `layersNoteEl` (R:484).
pub const LAYERS_NOTE_EL: &str = "layers-note";
/// `layersEmptyEl` (R:485).
pub const LAYERS_EMPTY_EL: &str = "layers-empty";
/// `linksEmptyEl` (R:486).
pub const LINKS_EMPTY_EL: &str = "links-empty";
/// `outlineEmptyEl` (R:487).
pub const OUTLINE_EMPTY_EL: &str = "outline-empty";
/// `pagesEmptyEl` (R:488).
pub const PAGES_EMPTY_EL: &str = "pages-empty";
/// `textEmptyEl` (R:489).
pub const TEXT_EMPTY_EL: &str = "text-empty";
/// `contentTargetsEl` (R:490).
pub const CONTENT_TARGETS_EL: &str = "content-targets";
/// `contentEmptyEl` (R:491).
pub const CONTENT_EMPTY_EL: &str = "content-empty";
/// `contentRefreshButton` (R:492).
pub const CONTENT_REFRESH_BUTTON: &str = "content-refresh";
/// `contentAddArmButton` (R:493).
pub const CONTENT_ADD_ARM_BUTTON: &str = "content-add-arm";
/// `textAddToolButton` (R:495).
pub const TEXT_ADD_TOOL_BUTTON: &str = "text-add-tool";
/// `textboxToolButton` (R:496).
pub const TEXTBOX_TOOL_BUTTON: &str = "textbox-tool";
/// `toolChip` (R:497).
pub const TOOL_CHIP: &str = "tool-chip";
/// `signChoiceDialog` (R:499).
pub const SIGN_CHOICE_DIALOG: &str = "sign-choice-dialog";
/// `signChoicePicture` (R:500).
pub const SIGN_CHOICE_PICTURE: &str = "sign-choice-picture";
/// `signChoiceCertificate` (R:501).
pub const SIGN_CHOICE_CERTIFICATE: &str = "sign-choice-certificate";
/// `signChoiceCancel` (R:502).
pub const SIGN_CHOICE_CANCEL: &str = "sign-choice-cancel";
/// `toolChipText` (R:505).
pub const TOOL_CHIP_TEXT: &str = "tool-chip-text";
/// `toolChipStop` (R:506).
pub const TOOL_CHIP_STOP: &str = "tool-chip-stop";
/// `contentAddFont` (R:507).
pub const CONTENT_ADD_FONT: &str = "content-add-font";
/// `contentAddSize` (R:508).
pub const CONTENT_ADD_SIZE: &str = "content-add-size";
/// `contentAddColor` (R:509).
pub const CONTENT_ADD_COLOR: &str = "content-add-color";
/// `contentAddText` (R:510).
pub const CONTENT_ADD_TEXT: &str = "content-add-text";
/// `contentAddGo` (R:511).
pub const CONTENT_ADD_GO: &str = "content-add-go";
/// `contentForm` (R:512).
pub const CONTENT_FORM: &str = "content-edit-form";
/// `contentEditLabel` (R:513).
pub const CONTENT_EDIT_LABEL: &str = "content-edit-label";
/// `contentNewText` (R:514).
pub const CONTENT_NEW_TEXT: &str = "content-new-text";
/// `contentHint` (R:515).
pub const CONTENT_HINT: &str = "content-hint";
/// `contentRepertoire` (R:517).
pub const CONTENT_REPERTOIRE: &str = "content-repertoire";
/// `contentDeleteButton` (R:518).
pub const CONTENT_DELETE_BUTTON: &str = "content-delete";
/// `contentCancelButton` (R:519).
pub const CONTENT_CANCEL_BUTTON: &str = "content-cancel";
/// `contentObjectForm` (R:521).
pub const CONTENT_OBJECT_FORM: &str = "content-object-form";
/// `contentObjectLabel` (R:522).
pub const CONTENT_OBJECT_LABEL: &str = "content-object-label";
/// `contentBoxX` (R:523).
pub const CONTENT_BOX_X: &str = "content-box-x";
/// `contentBoxY` (R:524).
pub const CONTENT_BOX_Y: &str = "content-box-y";
/// `contentBoxW` (R:525).
pub const CONTENT_BOX_W: &str = "content-box-w";
/// `contentBoxH` (R:526).
pub const CONTENT_BOX_H: &str = "content-box-h";
/// `contentObjectHint` (R:527).
pub const CONTENT_OBJECT_HINT: &str = "content-object-hint";
/// `contentReplaceButton` (R:528).
pub const CONTENT_REPLACE_BUTTON: &str = "content-replace";
/// `contentReplacePanel` (R:530).
pub const CONTENT_REPLACE_PANEL: &str = "content-find-replace";
/// `contentFindInput` (R:531).
pub const CONTENT_FIND_INPUT: &str = "content-find";
/// `contentReplaceWith` (R:532).
pub const CONTENT_REPLACE_WITH: &str = "content-replace-with";
/// `contentScopePage` (R:533).
pub const CONTENT_SCOPE_PAGE: &str = "content-scope-page";
/// `contentScopeRun` (R:534).
pub const CONTENT_SCOPE_RUN: &str = "content-scope-run";
/// `contentPreviewButton` (R:535).
pub const CONTENT_PREVIEW_BUTTON: &str = "content-preview-button";
/// `contentMatchesEl` (R:536).
pub const CONTENT_MATCHES_EL: &str = "content-matches";
/// `contentReplaceSummary` (R:537).
pub const CONTENT_REPLACE_SUMMARY: &str = "content-replace-summary";
/// `contentReplaceApply` (R:538).
pub const CONTENT_REPLACE_APPLY: &str = "content-replace-apply";
/// `contentReplaceCancel` (R:539).
pub const CONTENT_REPLACE_CANCEL: &str = "content-replace-cancel";
/// `contentDeleteObjectButton` (R:540).
pub const CONTENT_DELETE_OBJECT_BUTTON: &str = "content-delete-object";
/// `contentCancelObjectButton` (R:541).
pub const CONTENT_CANCEL_OBJECT_BUTTON: &str = "content-cancel-object";
/// `textPanelEl` (R:550).
pub const TEXT_PANEL_EL: &str = "text-panel";
/// `documentTitleEl` (R:555).
pub const DOCUMENT_TITLE_EL: &str = "document-title";
/// `dirtyChipEl` (R:556).
pub const DIRTY_CHIP_EL: &str = "dirty-chip";
/// `inspectorEl` (R:557).
pub const INSPECTOR_EL: &str = "inspector";
/// `inspectorToggle` (R:558).
pub const INSPECTOR_TOGGLE: &str = "inspector-toggle";
/// `newImagesButton` (R:603).
pub const NEW_IMAGES_BUTTON: &str = "new-images";
/// `exportButton` (R:604).
pub const EXPORT_BUTTON: &str = "export";
/// `exportDialog` (R:605).
pub const EXPORT_DIALOG: &str = "export-dialog";
/// `exportKindSelect` (R:606).
pub const EXPORT_KIND_SELECT: &str = "export-kind";
/// `exportDpiSelect` (R:607).
pub const EXPORT_DPI_SELECT: &str = "export-dpi";
/// `exportStartButton` (R:608).
pub const EXPORT_START_BUTTON: &str = "export-start";
/// `exportCancelButton` (R:609).
pub const EXPORT_CANCEL_BUTTON: &str = "export-cancel";
/// `insertButton` (R:610).
pub const INSERT_BUTTON: &str = "insert-pages";
/// `extractButton` (R:611).
pub const EXTRACT_BUTTON: &str = "extract-pages";
/// `insertDialog` (R:612).
pub const INSERT_DIALOG: &str = "insert-dialog";
/// `insertSourceEl` (R:613).
pub const INSERT_SOURCE_EL: &str = "insert-source";
/// `insertAfterInput` (R:614).
pub const INSERT_AFTER_INPUT: &str = "insert-after";
/// `insertRangeInput` (R:615).
pub const INSERT_RANGE_INPUT: &str = "insert-range";
/// `insertStartButton` (R:616).
pub const INSERT_START_BUTTON: &str = "insert-start";
/// `insertCancelButton` (R:617).
pub const INSERT_CANCEL_BUTTON: &str = "insert-cancel";
/// `extractDialog` (R:618).
pub const EXTRACT_DIALOG: &str = "extract-dialog";
/// `extractRangeInput` (R:619).
pub const EXTRACT_RANGE_INPUT: &str = "extract-range";
/// `extractStartButton` (R:620).
pub const EXTRACT_START_BUTTON: &str = "extract-start";
/// `extractCancelButton` (R:621).
pub const EXTRACT_CANCEL_BUTTON: &str = "extract-cancel";
/// `securityButton` (R:622).
pub const SECURITY_BUTTON: &str = "security";
/// `certSignOpenButton` (R:623).
pub const CERT_SIGN_OPEN_BUTTON: &str = "cert-sign-open";
/// `certSignDialog` (R:624).
pub const CERT_SIGN_DIALOG: &str = "cert-sign-dialog";
/// `certSignPickButton` (R:625).
pub const CERT_SIGN_PICK_BUTTON: &str = "cert-sign-pick";
/// `certSignFileEl` (R:626).
pub const CERT_SIGN_FILE_EL: &str = "cert-sign-file";
/// `certSignPassword` (R:627).
pub const CERT_SIGN_PASSWORD: &str = "cert-sign-password";
/// `certSignPage` (R:628).
pub const CERT_SIGN_PAGE: &str = "cert-sign-page";
/// `certSignVisible` (R:629).
pub const CERT_SIGN_VISIBLE: &str = "cert-sign-visible";
/// `certSignBox` (R:630).
pub const CERT_SIGN_BOX: &str = "cert-sign-box";
/// `certSignX` (R:631).
pub const CERT_SIGN_X: &str = "cert-sign-x";
/// `certSignY` (R:632).
pub const CERT_SIGN_Y: &str = "cert-sign-y";
/// `certSignW` (R:633).
pub const CERT_SIGN_W: &str = "cert-sign-w";
/// `certSignH` (R:634).
pub const CERT_SIGN_H: &str = "cert-sign-h";
/// `certSignReason` (R:635).
pub const CERT_SIGN_REASON: &str = "cert-sign-reason";
/// `certSignCertify` (R:636).
pub const CERT_SIGN_CERTIFY: &str = "cert-sign-certify";
/// `certSignCertifyReason` (R:637).
pub const CERT_SIGN_CERTIFY_REASON: &str = "cert-sign-certify-reason";
/// `certSignStatus` (R:638).
pub const CERT_SIGN_STATUS: &str = "cert-sign-status";
/// `certSignFieldEl` (R:639).
pub const CERT_SIGN_FIELD_EL: &str = "cert-sign-field";
/// `certSignGo` (R:640).
pub const CERT_SIGN_GO: &str = "cert-sign-go";
/// `certSignCancel` (R:641).
pub const CERT_SIGN_CANCEL: &str = "cert-sign-cancel";
/// `verifyDialog` (R:642).
pub const VERIFY_DIALOG: &str = "verify-dialog";
/// `verifyOpenButton` (R:643).
pub const VERIFY_OPEN_BUTTON: &str = "verify-open";
/// `verifySummary` (R:644).
pub const VERIFY_SUMMARY: &str = "verify-summary";
/// `verifyList` (R:645).
pub const VERIFY_LIST: &str = "verify-list";
/// `verifyCertified` (R:646).
pub const VERIFY_CERTIFIED: &str = "verify-certified";
/// `verifyTrusted` (R:647).
pub const VERIFY_TRUSTED: &str = "verify-trusted";
/// `verifyTrustAdd` (R:648).
pub const VERIFY_TRUST_ADD: &str = "verify-trust-add";
/// `verifyTrustClear` (R:649).
pub const VERIFY_TRUST_CLEAR: &str = "verify-trust-clear";
/// `verifyRefresh` (R:650).
pub const VERIFY_REFRESH: &str = "verify-refresh";
/// `verifyClose` (R:651).
pub const VERIFY_CLOSE: &str = "verify-close";
/// `batchDialog` (R:653).
pub const BATCH_DIALOG: &str = "batch-dialog";
/// `readerEl` (R:655).
pub const READER_EL: &str = "reader";
/// `compareView` (R:656).
pub const COMPARE_VIEW: &str = "compare-view";
/// `compareCanvasA` (R:657).
pub const COMPARE_CANVAS_A: &str = "compare-canvas-a";
/// `compareCanvasB` (R:658).
pub const COMPARE_CANVAS_B: &str = "compare-canvas-b";
/// `compareOverlay` (R:659).
pub const COMPARE_OVERLAY: &str = "compare-overlay";
/// `compareShowDiffInput` (R:660).
pub const COMPARE_SHOW_DIFF_INPUT: &str = "compare-show-diff";
/// `compareSummaryEl` (R:661).
pub const COMPARE_SUMMARY_EL: &str = "compare-summary";
/// `compareListEl` (R:662).
pub const COMPARE_LIST_EL: &str = "compare-list";
/// `comparePageBInput` (R:663).
pub const COMPARE_PAGE_BINPUT: &str = "compare-page-b";
/// `compareCloseButton` (R:664).
pub const COMPARE_CLOSE_BUTTON: &str = "compare-close";
/// `compareNameA` (R:665).
pub const COMPARE_NAME_A: &str = "compare-name-a";
/// `compareNameB` (R:666).
pub const COMPARE_NAME_B: &str = "compare-name-b";
/// `batchFolderInput` (R:681).
pub const BATCH_FOLDER_INPUT: &str = "batch-folder";
/// `batchPickButton` (R:682).
pub const BATCH_PICK_BUTTON: &str = "batch-pick";
/// `batchKindSelect` (R:683).
pub const BATCH_KIND_SELECT: &str = "batch-kind";
/// `batchTextInput` (R:684).
pub const BATCH_TEXT_INPUT: &str = "batch-text";
/// `batchPagesInput` (R:685).
pub const BATCH_PAGES_INPUT: &str = "batch-pages";
/// `batchAlignSelect` (R:686).
pub const BATCH_ALIGN_SELECT: &str = "batch-align";
/// `batchBatesStartInput` (R:687).
pub const BATCH_BATES_START_INPUT: &str = "batch-bates-start";
/// `batchBatesPadInput` (R:688).
pub const BATCH_BATES_PAD_INPUT: &str = "batch-bates-pad";
/// `batchSummaryEl` (R:689).
pub const BATCH_SUMMARY_EL: &str = "batch-summary";
/// `batchListEl` (R:690).
pub const BATCH_LIST_EL: &str = "batch-list";
/// `batchStartButton` (R:691).
pub const BATCH_START_BUTTON: &str = "batch-start";
/// `batchCancelButton` (R:692).
pub const BATCH_CANCEL_BUTTON: &str = "batch-cancel";
/// `batchCloseButton` (R:693).
pub const BATCH_CLOSE_BUTTON: &str = "batch-close";
/// `securityDialog` (R:711).
pub const SECURITY_DIALOG: &str = "security-dialog";
/// `securityCurrentEl` (R:712).
pub const SECURITY_CURRENT_EL: &str = "security-current";
/// `securityModeNoneInput` (R:713).
pub const SECURITY_MODE_NONE_INPUT: &str = "security-mode-none";
/// `securityModeAesInput` (R:714).
pub const SECURITY_MODE_AES_INPUT: &str = "security-mode-aes";
/// `securityUserInput` (R:715).
pub const SECURITY_USER_INPUT: &str = "security-user";
/// `securityOwnerInput` (R:716).
pub const SECURITY_OWNER_INPUT: &str = "security-owner";
/// `securityStatusEl` (R:717).
pub const SECURITY_STATUS_EL: &str = "security-status";
/// `securityApplyButton` (R:718).
pub const SECURITY_APPLY_BUTTON: &str = "security-apply";
/// `securityCancelButton` (R:719).
pub const SECURITY_CANCEL_BUTTON: &str = "security-cancel";
/// `startScreen` (R:939).
pub const START_SCREEN: &str = "start-screen";
/// `startOpenButton` (R:940).
pub const START_OPEN_BUTTON: &str = "start-open";
/// `startNewButton` (R:941).
pub const START_NEW_BUTTON: &str = "start-new";
/// `startImagesButton` (R:942).
pub const START_IMAGES_BUTTON: &str = "start-images";
/// `startRecent` (R:943).
pub const START_RECENT: &str = "start-recent";
/// `startRecentList` (R:944).
pub const START_RECENT_LIST: &str = "start-recent-list";
/// `startRecentClear` (R:945).
pub const START_RECENT_CLEAR: &str = "start-recent-clear";
/// `pausedBar` (R:1328).
pub const PAUSED_BAR: &str = "page-paused";
/// `pausedBarText` (R:1329).
pub const PAUSED_BAR_TEXT: &str = "page-paused-text";
/// `pausedRetry` (R:1330).
pub const PAUSED_RETRY: &str = "page-retry";

/// `objectMenuButtons` (R:273): menu item key → id, in DOM order.
pub const OBJECT_MENU_BUTTONS: [(&str, &str); 12] = [
    ("cut", "object-menu-cut"),
    ("copy", "object-menu-copy"),
    ("paste", "object-menu-paste"),
    ("replace", "object-menu-replace"),
    ("duplicate", "object-menu-duplicate"),
    ("rotateCw", "object-menu-rotate-cw"),
    ("rotateCcw", "object-menu-rotate-ccw"),
    ("flipH", "object-menu-flip-h"),
    ("flipV", "object-menu-flip-v"),
    ("crop", "object-menu-crop"),
    ("delete", "object-menu-delete"),
    ("redactPage", "object-menu-redact-page"),
];
/// `markupButtons` (R:312): (id, annotation type, announcement).
pub const MARKUP_BUTTONS: [(&str, &str, &str); 3] = [
    ("markup-highlight", "Highlight", "Highlighted match"),
    ("markup-underline", "Underline", "Underlined match"),
    ("markup-strikeout", "StrikeOut", "Struck out match"),
];
/// `shapeButtons` (R:320): (id, annotation type, announcement).
pub const SHAPE_BUTTONS: [(&str, &str, &str); 3] = [
    ("shape-square", "Square", "Added square"),
    ("shape-circle", "Circle", "Added circle"),
    ("shape-line", "Line", "Added line"),
];
/// `pageToolButtons` (R:453): (id, Page menu op).
pub const PAGE_TOOL_BUTTONS: [(&str, &str); 5] = [
    ("page-rotate-right", "rotate-right"),
    ("page-rotate-left", "rotate-left"),
    ("page-move-up", "move-up"),
    ("page-move-down", "move-down"),
    ("page-delete", "delete-page"),
];
/// `contentTransformButtons` (R:543): key → id.
pub const CONTENT_TRANSFORM_BUTTONS: [(&str, &str); 5] = [
    ("rotateCcw", "content-rotate-ccw"),
    ("rotateCw", "content-rotate-cw"),
    ("flipH", "content-flip-h"),
    ("flipV", "content-flip-v"),
    ("crop", "content-crop"),
];
/// `modeInputs` (R:561): (radio id, mode, context row id).
pub const MODE_INPUTS: [(&str, UiMode, &str); 6] = [
    ("mode-read", UiMode::Read, "context-read"),
    ("mode-annotate", UiMode::Annotate, "context-annotate"),
    ("mode-organize", UiMode::Organize, "context-organize"),
    ("mode-forms", UiMode::Forms, "context-forms"),
    ("mode-edit", UiMode::Edit, "context-edit"),
    ("mode-redact", UiMode::Redact, "context-redact"),
];
/// `sidebarTabs` (R:569).
pub const SIDEBAR_TABS: [&str; 2] = ["tab-pages", "tab-bookmarks"];
/// `sidebarPanels` (R:573).
pub const SIDEBAR_PANELS: [&str; 2] = ["panel-pages", "panel-bookmarks"];
/// `inspectorTabs` (R:577).
pub const INSPECTOR_TABS: [&str; 5] = [
    "tab-notes",
    "tab-fields",
    "tab-links",
    "tab-text",
    "tab-layers",
];
/// `inspectorPanels` (R:584).
pub const INSPECTOR_PANELS: [&str; 5] = [
    "panel-notes",
    "panel-fields",
    "panel-links",
    "text-panel",
    "panel-layers",
];
/// `permissionBoxes` (R:721): (permission, checkbox id), in the engine's fixed order.
pub const PERMISSION_BOXES: [(&str, &str); 5] = [
    ("print", "security-perm-print"),
    ("copy", "security-perm-copy"),
    ("edit", "security-perm-edit"),
    ("annotate", "security-perm-annotate"),
    ("form", "security-perm-form"),
];
/// `document.getElementById(`content-match-${index}`)` (R:6211): a replace-preview match checkbox.
pub fn content_match(index: usize) -> String {
    format!("content-match-{index}")
}

/// The element constants of `createRedactUi` (redact-ui.ts).
pub mod redact_ui {
    /// `pageRegion` (U:172).
    pub const PAGE_REGION: &str = "page";
    /// `chip` (U:173).
    pub const CHIP: &str = "redact-chip";
    /// `underneathInput` (U:174).
    pub const UNDERNEATH_INPUT: &str = "redact-underneath";
    /// `findOpen` (U:175).
    pub const FIND_OPEN: &str = "redact-find-open";
    /// `panel` (U:176).
    pub const PANEL: &str = "redact-panel";
    /// `heading` (U:177).
    pub const HEADING: &str = "redact-panel-heading";
    /// `applyOpen` (U:178).
    pub const APPLY_OPEN: &str = "redact-apply-open";
    /// `countEl` (U:179).
    pub const COUNT_EL: &str = "redact-count";
    /// `emptyEl` (U:180).
    pub const EMPTY_EL: &str = "redact-panel-empty";
    /// `flattenNoteEl` (U:181).
    pub const FLATTEN_NOTE_EL: &str = "redact-flatten-note";
    /// `listEl` (U:182).
    pub const LIST_EL: &str = "redact-list";
    /// `findDialog` (U:183).
    pub const FIND_DIALOG: &str = "redact-find-dialog";
    /// `searchText` (U:184).
    pub const SEARCH_TEXT: &str = "redact-search-text";
    /// `searchGo` (U:185).
    pub const SEARCH_GO: &str = "redact-search-go";
    /// `findCancel` (U:186).
    pub const FIND_CANCEL: &str = "redact-find-cancel";
    /// `confirmDialog` (U:187).
    pub const CONFIRM_DIALOG: &str = "redact-dialog";
    /// `confirmHeadingEl` (U:188).
    pub const CONFIRM_HEADING_EL: &str = "redact-heading";
    /// `sharedEl` (U:189).
    pub const SHARED_EL: &str = "redact-shared";
    /// `hiddenInput` (U:190).
    pub const HIDDEN_INPUT: &str = "redact-hidden";
    /// `detailsEl` (U:191).
    pub const DETAILS_EL: &str = "redact-details";
    /// `keptEl` (U:192).
    pub const KEPT_EL: &str = "redact-kept";
    /// `previewTable` (U:193).
    pub const PREVIEW_TABLE: &str = "redact-preview";
    /// `limitEl` (U:194).
    pub const LIMIT_EL: &str = "redact-limit";
    /// `confirmButton` (U:195).
    pub const CONFIRM_BUTTON: &str = "redact-apply";
    /// `confirmCancel` (U:196).
    pub const CONFIRM_CANCEL: &str = "redact-cancel";
    /// `pendingDialog` (U:197).
    pub const PENDING_DIALOG: &str = "redact-pending-dialog";
    /// `pendingHeading` (U:198).
    pub const PENDING_HEADING: &str = "redact-pending-heading";
    /// `pendingTextEl` (U:199).
    pub const PENDING_TEXT_EL: &str = "redact-pending-text";
    /// `pendingNow` (U:200).
    pub const PENDING_NOW: &str = "redact-pending-now";
    /// `pendingMarks` (U:201).
    pub const PENDING_MARKS: &str = "redact-pending-marks";
    /// `pendingCancel` (U:202).
    pub const PENDING_CANCEL: &str = "redact-pending-cancel";
    /// `askDialog` (U:203).
    pub const ASK_DIALOG: &str = "redact-ask-dialog";
    /// `askHeading` (U:204).
    pub const ASK_HEADING: &str = "redact-ask-heading";
    /// `askText` (U:205).
    pub const ASK_TEXT: &str = "redact-ask-text";
    /// `askYes` (U:206).
    pub const ASK_YES: &str = "redact-ask-yes";
    /// `askAlt` (U:207).
    pub const ASK_ALT: &str = "redact-ask-alt";
    /// `askCancel` (U:208).
    pub const ASK_CANCEL: &str = "redact-ask-cancel";
    /// `banner` (U:209).
    pub const BANNER: &str = "redact-banner";
    /// `bannerTextEl` (U:210).
    pub const BANNER_TEXT_EL: &str = "redact-banner-text";
    /// `bannerOriginal` (U:211).
    pub const BANNER_ORIGINAL: &str = "redact-banner-original";
    /// `bannerDismiss` (U:212).
    pub const BANNER_DISMISS: &str = "redact-banner-dismiss";
    /// `sanitizeBoxes` (U:213): switch key → checkbox id (`redact-sanitize-${key}`), SANITIZE_KEYS order.
    pub const SANITIZE_BOXES: [(&str, &str); 9] = [
        ("info", "redact-sanitize-info"),
        ("xmp", "redact-sanitize-xmp"),
        ("attachments", "redact-sanitize-attachments"),
        ("outlines", "redact-sanitize-outlines"),
        ("annots", "redact-sanitize-annots"),
        ("links", "redact-sanitize-links"),
        ("fields", "redact-sanitize-fields"),
        ("layers", "redact-sanitize-layers"),
        ("struct", "redact-sanitize-struct"),
    ];
}

/// Every single-element constant as (TS name, id): renderer.ts, then redact-ui.ts (prefixed `redactUi.`).
pub const NAMES: &[(&str, &str)] = &[
    ("statusEl", "status"),
    ("errorEl", "error"),
    ("bannerEl", "banner"),
    ("bannerTextEl", "banner-text"),
    ("bannerDismissEl", "banner-dismiss"),
    ("warningEl", "warning"),
    ("canvas", "canvas"),
    ("overlay", "text-layer"),
    ("hitLayer", "hit-layer"),
    ("fieldBar", "field-bar"),
    ("fieldCount", "field-count"),
    ("fieldHighlight", "field-highlight"),
    ("fieldNext", "field-next"),
    ("extracted", "extracted"),
    ("caption", "caption"),
    ("viewport", "viewport"),
    ("pageRegion", "page"),
    ("objectMenu", "object-menu"),
    ("pageInput", "page-input"),
    ("prevButton", "prev"),
    ("nextButton", "next"),
    ("openButton", "open"),
    ("saveButton", "save"),
    ("saveCopyButton", "save-copy"),
    ("undoButton", "undo"),
    ("redoButton", "redo"),
    ("editStatusEl", "edit-status"),
    ("notesHeading", "notes-heading"),
    ("notesEmptyEl", "notes-empty"),
    ("noteAddButton", "note-add"),
    ("markupAllButton", "markup-highlight-all"),
    ("freeTextAddButton", "freetext-add"),
    ("inkDrawButton", "ink-draw"),
    ("stampIconButton", "stamp-icon"),
    ("signButton", "sign-add"),
    ("stampIconDialog", "stamp-icon-dialog"),
    ("stampIconSaveButton", "stamp-icon-save"),
    ("stampIconCancelButton", "stamp-icon-cancel"),
    ("noteListEl", "note-list"),
    ("noteDialog", "note-dialog"),
    ("noteDialogHeading", "note-dialog-heading"),
    ("noteTextInput", "note-text"),
    ("noteSaveButton", "note-save"),
    ("noteCancelButton", "note-cancel"),
    ("zoomOutButton", "zoom-out"),
    ("zoomInButton", "zoom-in"),
    ("fitWidthButton", "fit-width"),
    ("fitPageButton", "fit-page"),
    ("zoom100Button", "zoom-100"),
    ("zoomLevel", "zoom-level"),
    ("searchInput", "search-input"),
    ("searchPrevButton", "search-prev"),
    ("searchNextButton", "search-next"),
    ("searchStatusEl", "search-status"),
    ("thumbsEl", "thumbs"),
    ("thumbsToggle", "thumbs-toggle"),
    ("sidebarEl", "page-sidebar"),
    ("dropHint", "drop-hint"),
    ("passwordForm", "password-form"),
    ("passwordName", "password-name"),
    ("passwordInput", "password-input"),
    ("passwordErrorEl", "password-error"),
    ("printButton", "print"),
    ("printDialog", "print-dialog"),
    ("printRangeInput", "print-range"),
    ("printStatusEl", "print-status"),
    ("printStartButton", "print-start"),
    ("printCancelButton", "print-cancel"),
    ("printPagesEl", "print-pages"),
    ("pageStatusEl", "page-status"),
    ("fieldsEl", "fields"),
    ("fieldsHeading", "fields-heading"),
    ("fieldsEmptyEl", "fields-empty"),
    ("fillSignButton", "fill-sign"),
    ("fillSignDrawButton", "fill-sign-draw"),
    ("sigpadDialog", "sigpad-dialog"),
    ("sigpadCanvas", "sigpad-canvas"),
    ("sigpadStatus", "sigpad-status"),
    ("sigpadClearButton", "sigpad-clear"),
    ("sigpadImageButton", "sigpad-image"),
    ("sigpadRemoveButton", "sigpad-remove"),
    ("sigpadSaveButton", "sigpad-save"),
    ("sigpadCancelButton", "sigpad-cancel"),
    ("fieldsStatusEl", "fields-status"),
    ("fieldsWarningEl", "fields-warning"),
    ("fieldListEl", "field-list"),
    ("designerEl", "field-designer"),
    ("designerHintEl", "field-designer-hint"),
    ("fieldNewKindEl", "field-new-kind"),
    ("fieldNewNameEl", "field-new-name"),
    ("fieldNewXEl", "field-new-x"),
    ("fieldNewYEl", "field-new-y"),
    ("fieldNewWEl", "field-new-w"),
    ("fieldNewHEl", "field-new-h"),
    ("fieldNewRectEl", "field-new-rect"),
    ("fieldNewTextEl", "field-new-text"),
    ("fieldNewComboEl", "field-new-combo"),
    ("fieldNewRadioEl", "field-new-radio"),
    ("fieldNewMaxLenEl", "field-new-maxlen"),
    ("fieldNewMultilineEl", "field-new-multiline"),
    ("fieldNewOptionsEl", "field-new-options"),
    ("fieldNewKidsEl", "field-new-kids"),
    ("fieldNewLabelEl", "field-new-label"),
    ("fieldNewReadOnlyEl", "field-new-readonly"),
    ("fieldNewRequiredEl", "field-new-required"),
    ("fieldNewDefaultTextEl", "field-new-default"),
    ("fieldNewDefaultBoolEl", "field-new-default-check"),
    ("fieldNewAddEl", "field-new-add"),
    ("fieldSelectEl", "field-select"),
    ("fieldPropNameEl", "field-prop-name"),
    ("fieldPropXEl", "field-prop-x"),
    ("fieldPropYEl", "field-prop-y"),
    ("fieldPropWEl", "field-prop-w"),
    ("fieldPropHEl", "field-prop-h"),
    ("fieldPropRectEl", "field-prop-rect"),
    ("fieldPropTextEl", "field-prop-text"),
    ("fieldPropComboEl", "field-prop-combo"),
    ("fieldPropMaxLenEl", "field-prop-maxlen"),
    ("fieldPropMultilineEl", "field-prop-multiline"),
    ("fieldPropOptionsEl", "field-prop-options"),
    ("fieldPropLabelEl", "field-prop-label"),
    ("fieldPropReadOnlyEl", "field-prop-readonly"),
    ("fieldPropRequiredEl", "field-prop-required"),
    ("fieldPropDefaultTextEl", "field-prop-default"),
    ("fieldPropDefaultBoolEl", "field-prop-default-check"),
    ("fieldPropApplyEl", "field-prop-apply"),
    ("fieldPropDeleteEl", "field-prop-delete"),
    ("helpDialog", "help-dialog"),
    ("helpMatrixEl", "help-matrix"),
    ("helpCloseEl", "help-close"),
    ("redactAreaButton", "redact-area"),
    ("redactDialog", "redact-dialog"),
    ("outlineListEl", "outline-list"),
    ("bookmarkAddButton", "bookmark-add"),
    ("linksHeading", "links-heading"),
    ("linkListEl", "link-list"),
    ("linkPageAddButton", "link-page-add"),
    ("linkUriAddButton", "link-uri-add"),
    ("navStatusEl", "nav-status"),
    ("bookmarkDialog", "bookmark-dialog"),
    ("bookmarkDialogHeading", "bookmark-dialog-heading"),
    ("bookmarkTitleInput", "bookmark-title"),
    ("bookmarkSaveButton", "bookmark-save"),
    ("bookmarkCancelButton", "bookmark-cancel"),
    ("linkDialog", "link-dialog"),
    ("linkDialogHeading", "link-dialog-heading"),
    ("linkTargetLabel", "link-target-label"),
    ("linkTargetInput", "link-target"),
    ("linkSaveButton", "link-save"),
    ("linkCancelButton", "link-cancel"),
    ("linkKindSelect", "link-kind"),
    ("linkDrawButton", "link-draw"),
    ("sigfieldDrawButton", "sigfield-draw"),
    ("editOutlinesInput", "edit-outlines"),
    ("pageLabelEl", "page-label"),
    ("watermarkButton", "stamp-watermark"),
    ("headerButton", "stamp-header"),
    ("pageLabelsButton", "page-labels"),
    ("stampDialog", "stamp-dialog"),
    ("stampDialogHeading", "stamp-dialog-heading"),
    ("stampKindSelect", "stamp-kind"),
    ("stampTextInput", "stamp-text"),
    ("stampPagesInput", "stamp-pages"),
    ("stampAlignSelect", "stamp-align"),
    ("stampBatesStartInput", "stamp-bates-start"),
    ("stampBatesPadInput", "stamp-bates-pad"),
    ("stampSaveButton", "stamp-save"),
    ("stampCancelButton", "stamp-cancel"),
    ("labelsDialog", "labels-dialog"),
    ("labelsFromInput", "labels-from"),
    ("labelsStyleSelect", "labels-style"),
    ("labelsPrefixInput", "labels-prefix"),
    ("labelsStartInput", "labels-start"),
    ("labelsSaveButton", "labels-save"),
    ("labelsClearButton", "labels-clear"),
    ("labelsCancelButton", "labels-cancel"),
    ("layersEl", "layers"),
    ("layerListEl", "layer-list"),
    ("layersNoteEl", "layers-note"),
    ("layersEmptyEl", "layers-empty"),
    ("linksEmptyEl", "links-empty"),
    ("outlineEmptyEl", "outline-empty"),
    ("pagesEmptyEl", "pages-empty"),
    ("textEmptyEl", "text-empty"),
    ("contentTargetsEl", "content-targets"),
    ("contentEmptyEl", "content-empty"),
    ("contentRefreshButton", "content-refresh"),
    ("contentAddArmButton", "content-add-arm"),
    ("textAddToolButton", "text-add-tool"),
    ("textboxToolButton", "textbox-tool"),
    ("toolChip", "tool-chip"),
    ("signChoiceDialog", "sign-choice-dialog"),
    ("signChoicePicture", "sign-choice-picture"),
    ("signChoiceCertificate", "sign-choice-certificate"),
    ("signChoiceCancel", "sign-choice-cancel"),
    ("toolChipText", "tool-chip-text"),
    ("toolChipStop", "tool-chip-stop"),
    ("contentAddFont", "content-add-font"),
    ("contentAddSize", "content-add-size"),
    ("contentAddColor", "content-add-color"),
    ("contentAddText", "content-add-text"),
    ("contentAddGo", "content-add-go"),
    ("contentForm", "content-edit-form"),
    ("contentEditLabel", "content-edit-label"),
    ("contentNewText", "content-new-text"),
    ("contentHint", "content-hint"),
    ("contentRepertoire", "content-repertoire"),
    ("contentDeleteButton", "content-delete"),
    ("contentCancelButton", "content-cancel"),
    ("contentObjectForm", "content-object-form"),
    ("contentObjectLabel", "content-object-label"),
    ("contentBoxX", "content-box-x"),
    ("contentBoxY", "content-box-y"),
    ("contentBoxW", "content-box-w"),
    ("contentBoxH", "content-box-h"),
    ("contentObjectHint", "content-object-hint"),
    ("contentReplaceButton", "content-replace"),
    ("contentReplacePanel", "content-find-replace"),
    ("contentFindInput", "content-find"),
    ("contentReplaceWith", "content-replace-with"),
    ("contentScopePage", "content-scope-page"),
    ("contentScopeRun", "content-scope-run"),
    ("contentPreviewButton", "content-preview-button"),
    ("contentMatchesEl", "content-matches"),
    ("contentReplaceSummary", "content-replace-summary"),
    ("contentReplaceApply", "content-replace-apply"),
    ("contentReplaceCancel", "content-replace-cancel"),
    ("contentDeleteObjectButton", "content-delete-object"),
    ("contentCancelObjectButton", "content-cancel-object"),
    ("textPanelEl", "text-panel"),
    ("documentTitleEl", "document-title"),
    ("dirtyChipEl", "dirty-chip"),
    ("inspectorEl", "inspector"),
    ("inspectorToggle", "inspector-toggle"),
    ("newImagesButton", "new-images"),
    ("exportButton", "export"),
    ("exportDialog", "export-dialog"),
    ("exportKindSelect", "export-kind"),
    ("exportDpiSelect", "export-dpi"),
    ("exportStartButton", "export-start"),
    ("exportCancelButton", "export-cancel"),
    ("insertButton", "insert-pages"),
    ("extractButton", "extract-pages"),
    ("insertDialog", "insert-dialog"),
    ("insertSourceEl", "insert-source"),
    ("insertAfterInput", "insert-after"),
    ("insertRangeInput", "insert-range"),
    ("insertStartButton", "insert-start"),
    ("insertCancelButton", "insert-cancel"),
    ("extractDialog", "extract-dialog"),
    ("extractRangeInput", "extract-range"),
    ("extractStartButton", "extract-start"),
    ("extractCancelButton", "extract-cancel"),
    ("securityButton", "security"),
    ("certSignOpenButton", "cert-sign-open"),
    ("certSignDialog", "cert-sign-dialog"),
    ("certSignPickButton", "cert-sign-pick"),
    ("certSignFileEl", "cert-sign-file"),
    ("certSignPassword", "cert-sign-password"),
    ("certSignPage", "cert-sign-page"),
    ("certSignVisible", "cert-sign-visible"),
    ("certSignBox", "cert-sign-box"),
    ("certSignX", "cert-sign-x"),
    ("certSignY", "cert-sign-y"),
    ("certSignW", "cert-sign-w"),
    ("certSignH", "cert-sign-h"),
    ("certSignReason", "cert-sign-reason"),
    ("certSignCertify", "cert-sign-certify"),
    ("certSignCertifyReason", "cert-sign-certify-reason"),
    ("certSignStatus", "cert-sign-status"),
    ("certSignFieldEl", "cert-sign-field"),
    ("certSignGo", "cert-sign-go"),
    ("certSignCancel", "cert-sign-cancel"),
    ("verifyDialog", "verify-dialog"),
    ("verifyOpenButton", "verify-open"),
    ("verifySummary", "verify-summary"),
    ("verifyList", "verify-list"),
    ("verifyCertified", "verify-certified"),
    ("verifyTrusted", "verify-trusted"),
    ("verifyTrustAdd", "verify-trust-add"),
    ("verifyTrustClear", "verify-trust-clear"),
    ("verifyRefresh", "verify-refresh"),
    ("verifyClose", "verify-close"),
    ("batchDialog", "batch-dialog"),
    ("readerEl", "reader"),
    ("compareView", "compare-view"),
    ("compareCanvasA", "compare-canvas-a"),
    ("compareCanvasB", "compare-canvas-b"),
    ("compareOverlay", "compare-overlay"),
    ("compareShowDiffInput", "compare-show-diff"),
    ("compareSummaryEl", "compare-summary"),
    ("compareListEl", "compare-list"),
    ("comparePageBInput", "compare-page-b"),
    ("compareCloseButton", "compare-close"),
    ("compareNameA", "compare-name-a"),
    ("compareNameB", "compare-name-b"),
    ("batchFolderInput", "batch-folder"),
    ("batchPickButton", "batch-pick"),
    ("batchKindSelect", "batch-kind"),
    ("batchTextInput", "batch-text"),
    ("batchPagesInput", "batch-pages"),
    ("batchAlignSelect", "batch-align"),
    ("batchBatesStartInput", "batch-bates-start"),
    ("batchBatesPadInput", "batch-bates-pad"),
    ("batchSummaryEl", "batch-summary"),
    ("batchListEl", "batch-list"),
    ("batchStartButton", "batch-start"),
    ("batchCancelButton", "batch-cancel"),
    ("batchCloseButton", "batch-close"),
    ("securityDialog", "security-dialog"),
    ("securityCurrentEl", "security-current"),
    ("securityModeNoneInput", "security-mode-none"),
    ("securityModeAesInput", "security-mode-aes"),
    ("securityUserInput", "security-user"),
    ("securityOwnerInput", "security-owner"),
    ("securityStatusEl", "security-status"),
    ("securityApplyButton", "security-apply"),
    ("securityCancelButton", "security-cancel"),
    ("startScreen", "start-screen"),
    ("startOpenButton", "start-open"),
    ("startNewButton", "start-new"),
    ("startImagesButton", "start-images"),
    ("startRecent", "start-recent"),
    ("startRecentList", "start-recent-list"),
    ("startRecentClear", "start-recent-clear"),
    ("pausedBar", "page-paused"),
    ("pausedBarText", "page-paused-text"),
    ("pausedRetry", "page-retry"),
    ("redactUi.pageRegion", "page"),
    ("redactUi.chip", "redact-chip"),
    ("redactUi.underneathInput", "redact-underneath"),
    ("redactUi.findOpen", "redact-find-open"),
    ("redactUi.panel", "redact-panel"),
    ("redactUi.heading", "redact-panel-heading"),
    ("redactUi.applyOpen", "redact-apply-open"),
    ("redactUi.countEl", "redact-count"),
    ("redactUi.emptyEl", "redact-panel-empty"),
    ("redactUi.flattenNoteEl", "redact-flatten-note"),
    ("redactUi.listEl", "redact-list"),
    ("redactUi.findDialog", "redact-find-dialog"),
    ("redactUi.searchText", "redact-search-text"),
    ("redactUi.searchGo", "redact-search-go"),
    ("redactUi.findCancel", "redact-find-cancel"),
    ("redactUi.confirmDialog", "redact-dialog"),
    ("redactUi.confirmHeadingEl", "redact-heading"),
    ("redactUi.sharedEl", "redact-shared"),
    ("redactUi.hiddenInput", "redact-hidden"),
    ("redactUi.detailsEl", "redact-details"),
    ("redactUi.keptEl", "redact-kept"),
    ("redactUi.previewTable", "redact-preview"),
    ("redactUi.limitEl", "redact-limit"),
    ("redactUi.confirmButton", "redact-apply"),
    ("redactUi.confirmCancel", "redact-cancel"),
    ("redactUi.pendingDialog", "redact-pending-dialog"),
    ("redactUi.pendingHeading", "redact-pending-heading"),
    ("redactUi.pendingTextEl", "redact-pending-text"),
    ("redactUi.pendingNow", "redact-pending-now"),
    ("redactUi.pendingMarks", "redact-pending-marks"),
    ("redactUi.pendingCancel", "redact-pending-cancel"),
    ("redactUi.askDialog", "redact-ask-dialog"),
    ("redactUi.askHeading", "redact-ask-heading"),
    ("redactUi.askText", "redact-ask-text"),
    ("redactUi.askYes", "redact-ask-yes"),
    ("redactUi.askAlt", "redact-ask-alt"),
    ("redactUi.askCancel", "redact-ask-cancel"),
    ("redactUi.banner", "redact-banner"),
    ("redactUi.bannerTextEl", "redact-banner-text"),
    ("redactUi.bannerOriginal", "redact-banner-original"),
    ("redactUi.bannerDismiss", "redact-banner-dismiss"),
];

/// One element's initial state in index.html.
struct Initial {
    id: &'static str,
    text: &'static str,
    value: &'static str,
    hidden: bool,
    disabled: bool,
    open: bool,
    checked: bool,
    attrs: &'static [(&'static str, &'static str)],
}

/// Elements whose initial state differs from [`Node::default`], in document order. `text` is
/// the element's `textContent` (only for elements with no block children; select and textarea
/// give `value` instead); `checked` is the `checked` attribute or an aria-pressed/checked/selected
/// of "true".
const INITIAL: &[Initial] = &[
    Initial {
        id: "drop-hint",
        text: "Drop a PDF here to open it",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "document-title",
        text: "Tandem PDF",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "dirty-chip",
        text: "Edited — not saved",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "open",
        text: "Open…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Control+O Meta+O")],
    },
    Initial {
        id: "save",
        text: "Save",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Control+S Meta+S")],
    },
    Initial {
        id: "save-copy",
        text: "Save As…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            (
                "title",
                "Save under a new name; the window then edits the new file",
            ),
            ("aria-keyshortcuts", "Control+Shift+S Meta+Shift+S"),
        ],
    },
    Initial {
        id: "print",
        text: "Print…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Control+P Meta+P")],
    },
    Initial {
        id: "new-images",
        text: "New from images…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Alt+Control+N Alt+Meta+N")],
    },
    Initial {
        id: "export",
        text: "Export…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Shift+Control+E Shift+Meta+E")],
    },
    Initial {
        id: "undo",
        text: "Undo",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Control+Z Meta+Z")],
    },
    Initial {
        id: "redo",
        text: "Redo",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Control+Shift+Z Meta+Shift+Z")],
    },
    Initial {
        id: "search-input",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("maxlength", "256"),
            ("aria-keyshortcuts", "Control+F Meta+F"),
        ],
    },
    Initial {
        id: "search-status",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "search-prev",
        text: "‹",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("aria-label", "Previous match"),
            ("title", "Previous match (Shift+Enter)"),
        ],
    },
    Initial {
        id: "search-next",
        text: "›",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("aria-label", "Next match"),
            ("title", "Next match (Enter)"),
        ],
    },
    Initial {
        id: "page-nav",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Page navigation")],
    },
    Initial {
        id: "prev",
        text: "‹ Previous",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Previous page")],
    },
    Initial {
        id: "page-input",
        text: "",
        value: "1",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("min", "1")],
    },
    Initial {
        id: "page-label",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "next",
        text: "Next ›",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Next page")],
    },
    Initial {
        id: "mode-read",
        text: "",
        value: "read",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "mode-annotate",
        text: "",
        value: "annotate",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "mode-organize",
        text: "",
        value: "organize",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "mode-forms",
        text: "",
        value: "forms",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "mode-edit",
        text: "",
        value: "edit",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "mode-redact",
        text: "",
        value: "redact",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "context-read",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Reading tools")],
    },
    Initial {
        id: "context-annotate",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Annotate tools")],
    },
    Initial {
        id: "markup-highlight",
        text: "Highlight",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            (
                "title",
                "Highlight the selected text, or the current search match; Cmd/Ctrl+Shift+H",
            ),
            ("aria-keyshortcuts", "Control+Shift+H Meta+Shift+H"),
        ],
    },
    Initial {
        id: "markup-underline",
        text: "Underline",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[(
            "title",
            "Underline the selected text, or the current search match",
        )],
    },
    Initial {
        id: "markup-strikeout",
        text: "Strike out",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[(
            "title",
            "Strike out the selected text, or the current search match",
        )],
    },
    Initial {
        id: "markup-highlight-all",
        text: "Highlight all matches",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("title", "Highlight every match of the search")],
    },
    Initial {
        id: "note-add",
        text: "Add note",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            ("aria-pressed", "false"),
            (
                "title",
                "Press, then click the page where the note goes; Cmd/Ctrl+Shift+N places one at the page centre",
            ),
            ("aria-keyshortcuts", "Control+Shift+N Meta+Shift+N"),
        ],
    },
    Initial {
        id: "freetext-add",
        text: "Add text box",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            ("aria-pressed", "false"),
            (
                "title",
                "Press, then click the page where the text box goes; Cmd/Ctrl+Shift+L opens it at the page centre",
            ),
            ("aria-keyshortcuts", "Control+Shift+L Meta+Shift+L"),
        ],
    },
    Initial {
        id: "shape-square",
        text: "Square",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            (
                "title",
                "Square over the current search match, or at the page centre; Cmd/Ctrl+Shift+G",
            ),
            ("aria-keyshortcuts", "Control+Shift+G Meta+Shift+G"),
        ],
    },
    Initial {
        id: "shape-circle",
        text: "Circle",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            (
                "title",
                "Circle over the current search match, or at the page centre; Cmd/Ctrl+Shift+O",
            ),
            ("aria-keyshortcuts", "Control+Shift+O Meta+Shift+O"),
        ],
    },
    Initial {
        id: "shape-line",
        text: "Line",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            (
                "title",
                "Line across the current search match, or at the page centre; Cmd/Ctrl+Shift+Backslash",
            ),
            (
                "aria-keyshortcuts",
                "Control+Shift+Backslash Meta+Shift+Backslash",
            ),
        ],
    },
    Initial {
        id: "ink-draw",
        text: "Draw ink",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            ("aria-pressed", "false"),
            (
                "title",
                "Press, then drag on the page to draw one ink stroke; Cmd/Ctrl+Shift+I draws a stand-in zigzag",
            ),
            ("aria-keyshortcuts", "Control+Shift+I Meta+Shift+I"),
        ],
    },
    Initial {
        id: "stamp-icon",
        text: "Stamp…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            ("title", "Add an icon stamp; Cmd/Ctrl+Shift+M"),
            ("aria-keyshortcuts", "Control+Shift+M Meta+Shift+M"),
        ],
    },
    Initial {
        id: "sign-add",
        text: "Sign…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            (
                "title",
                "Add your signature, or sign digitally with a certificate",
            ),
            ("aria-keyshortcuts", "Control+Shift+J Meta+Shift+J"),
        ],
    },
    Initial {
        id: "context-organize",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Organize tools")],
    },
    Initial {
        id: "page-tools",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Page tools")],
    },
    Initial {
        id: "page-rotate-right",
        text: "Rotate right",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Shift+Control+] Shift+Meta+]")],
    },
    Initial {
        id: "page-rotate-left",
        text: "Rotate left",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Shift+Control+[ Shift+Meta+[")],
    },
    Initial {
        id: "page-move-up",
        text: "Move up",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Alt+Control+ArrowUp Alt+Meta+ArrowUp")],
    },
    Initial {
        id: "page-move-down",
        text: "Move down",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[(
            "aria-keyshortcuts",
            "Alt+Control+ArrowDown Alt+Meta+ArrowDown",
        )],
    },
    Initial {
        id: "page-delete",
        text: "Delete page",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[(
            "aria-keyshortcuts",
            "Shift+Control+Backspace Shift+Meta+Backspace",
        )],
    },
    Initial {
        id: "insert-pages",
        text: "Insert pages…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Alt+Control+I Alt+Meta+I")],
    },
    Initial {
        id: "extract-pages",
        text: "Extract pages…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Alt+Control+E Alt+Meta+E")],
    },
    Initial {
        id: "stamp-watermark",
        text: "Watermark…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Alt+Control+W Alt+Meta+W")],
    },
    Initial {
        id: "stamp-header",
        text: "Header/footer…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Alt+Control+F Alt+Meta+F")],
    },
    Initial {
        id: "page-labels",
        text: "Page labels…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Alt+Control+L Alt+Meta+L")],
    },
    Initial {
        id: "security",
        text: "Security…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Alt+Control+S Alt+Meta+S")],
    },
    Initial {
        id: "context-forms",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Form tools")],
    },
    Initial {
        id: "textbox-tool",
        text: "Add text box",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            ("aria-pressed", "false"),
            (
                "title",
                "Then click the page where the text goes; Esc stops",
            ),
        ],
    },
    Initial {
        id: "cert-sign-open",
        text: "Sign…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[(
            "title",
            "Add your signature, or sign digitally with a certificate",
        )],
    },
    Initial {
        id: "sigfield-draw",
        text: "Draw signature field",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            ("aria-pressed", "false"),
            ("title", "Drag on the page to place a signature field"),
        ],
    },
    Initial {
        id: "context-edit",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Text editing tools")],
    },
    Initial {
        id: "text-add-tool",
        text: "Add text",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            ("aria-pressed", "false"),
            (
                "title",
                "Then click empty space on the page to type new text there; Esc stops",
            ),
        ],
    },
    Initial {
        id: "edit-outlines",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[("aria-label", "Outline objects")],
    },
    Initial {
        id: "context-redact",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Redaction tools")],
    },
    Initial {
        id: "redact-chip",
        text: "Drag over anything you want removed. Esc to stop.",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-find-open",
        text: "Find text to redact…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-area",
        text: "Draw box",
        value: "",
        hidden: true,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-pressed", "false")],
    },
    Initial {
        id: "inspector-toggle",
        text: "Inspector",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-expanded", "true"), ("aria-controls", "inspector")],
    },
    Initial {
        id: "field-bar",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Form fields")],
    },
    Initial {
        id: "field-count",
        text: "This page has 0 form fields.",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "field-highlight",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[("aria-label", "Highlight fields")],
    },
    Initial {
        id: "field-next",
        text: "Next field",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "error",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-banner",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "redact-banner-original",
        text: "Open original",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-banner-dismiss",
        text: "×",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("aria-label", "Dismiss this notice"),
            ("title", "Dismiss this notice"),
        ],
    },
    Initial {
        id: "banner",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "banner-dismiss",
        text: "×",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("aria-label", "Dismiss this notice"),
            ("title", "Dismiss this notice"),
        ],
    },
    Initial {
        id: "warning",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "page-paused",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "page-retry",
        text: "Try again",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "password-form",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "password-input",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "1024")],
    },
    Initial {
        id: "password-submit",
        text: "Unlock",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "start-screen",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "start-heading")],
    },
    Initial {
        id: "start-heading",
        text: "Tandem PDF",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "start-open",
        text: "Open a PDF…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "start-new",
        text: "New blank PDF",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "start-images",
        text: "New PDF from images…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "start-recent",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "start-recent-heading")],
    },
    Initial {
        id: "start-recent-heading",
        text: "Recent files",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "start-recent-clear",
        text: "Clear recent files",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "start-drop",
        text: "Or drop a PDF anywhere in this window to open it.",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "page-sidebar",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Pages and bookmarks")],
    },
    Initial {
        id: "tab-pages",
        text: "Pages",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[("aria-selected", "true"), ("aria-controls", "panel-pages")],
    },
    Initial {
        id: "tab-bookmarks",
        text: "Bookmarks",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("aria-selected", "false"),
            ("aria-controls", "panel-bookmarks"),
        ],
    },
    Initial {
        id: "thumbs-toggle",
        text: "Thumbnails",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            ("aria-pressed", "false"),
            ("aria-controls", "panel-pages panel-bookmarks"),
            ("aria-keyshortcuts", "Control+Shift+T Meta+Shift+T"),
            ("title", "Collapse or expand the sidebar"),
        ],
    },
    Initial {
        id: "panel-pages",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "tab-pages")],
    },
    Initial {
        id: "thumbs",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Pages")],
    },
    Initial {
        id: "pages-empty",
        text: "No document open yet.",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "panel-bookmarks",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "tab-bookmarks")],
    },
    Initial {
        id: "outline",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "outline-heading")],
    },
    Initial {
        id: "outline-heading",
        text: "Bookmarks",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "bookmark-add",
        text: "Add bookmark",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-keyshortcuts", "Shift+Control+B Shift+Meta+B")],
    },
    Initial {
        id: "outline-empty",
        text: "No bookmarks in this document.",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "workspace",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Document")],
    },
    Initial {
        id: "tool-chip",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "tool-chip-stop",
        text: "Esc",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Stop this tool")],
    },
    Initial {
        id: "page",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[(
            "aria-label",
            "Rendered page. Use Page Up and Page Down, or Home and End, to change page.",
        )],
    },
    Initial {
        id: "canvas",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-hidden", "true")],
    },
    Initial {
        id: "text-layer",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-hidden", "true")],
    },
    Initial {
        id: "hit-layer",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Page editing targets")],
    },
    Initial {
        id: "object-menu",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Object actions")],
    },
    Initial {
        id: "object-menu-cut",
        text: "Cut",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "object-menu-copy",
        text: "Copy",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "object-menu-paste",
        text: "Paste",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "object-menu-replace",
        text: "Replace…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "object-menu-duplicate",
        text: "Duplicate",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "object-menu-rotate-cw",
        text: "Rotate clockwise",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "object-menu-rotate-ccw",
        text: "Rotate counter-clockwise",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "object-menu-flip-h",
        text: "Flip horizontal",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "object-menu-flip-v",
        text: "Flip vertical",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "object-menu-crop",
        text: "Crop",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-checked", "false")],
    },
    Initial {
        id: "object-menu-delete",
        text: "Delete",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "object-menu-redact-page",
        text: "Redact whole page",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "caption",
        text: "No page",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "compare-view",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Compare")],
    },
    Initial {
        id: "compare-page-b",
        text: "",
        value: "1",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("min", "1")],
    },
    Initial {
        id: "compare-close",
        text: "Close compare",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "compare-canvas-a",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-hidden", "true")],
    },
    Initial {
        id: "compare-canvas-b",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-hidden", "true")],
    },
    Initial {
        id: "compare-overlay",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-hidden", "true")],
    },
    Initial {
        id: "compare-list",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Lines only on one side")],
    },
    Initial {
        id: "redact-panel",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "redact-panel-heading")],
    },
    Initial {
        id: "redact-panel-heading",
        text: "Redactions (0)",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-apply-open",
        text: "Redact areas…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-count",
        text: "No boxes yet",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-panel-empty",
        text: "Drag over anything you want removed: text, pictures, drawings, comments or form fields. Right-click the page to redact the whole page.",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-flatten-note",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-list",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Boxes by page")],
    },
    Initial {
        id: "inspector",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Inspector")],
    },
    Initial {
        id: "tab-notes",
        text: "Comments",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-selected", "false"), ("aria-controls", "panel-notes")],
    },
    Initial {
        id: "tab-fields",
        text: "Fields",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("aria-selected", "false"),
            ("aria-controls", "panel-fields"),
        ],
    },
    Initial {
        id: "tab-links",
        text: "Links",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[("aria-selected", "true"), ("aria-controls", "panel-links")],
    },
    Initial {
        id: "tab-text",
        text: "Text",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-selected", "false"), ("aria-controls", "text-panel")],
    },
    Initial {
        id: "tab-layers",
        text: "Layers",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("aria-selected", "false"),
            ("aria-controls", "panel-layers"),
        ],
    },
    Initial {
        id: "panel-notes",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "tab-notes")],
    },
    Initial {
        id: "notes",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "notes-heading")],
    },
    Initial {
        id: "notes-heading",
        text: "Comments on page 1",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "notes-empty",
        text: "No annotations on this page yet.",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "panel-fields",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "tab-fields")],
    },
    Initial {
        id: "fields",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "fields-heading")],
    },
    Initial {
        id: "fields-heading",
        text: "Form fields on page 1",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "fields-warning",
        text: "Runs a script this app does not execute; the value is saved as typed.",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "fields-empty",
        text: "No fillable fields on this page.",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "fill-sign",
        text: "Sign…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            ("aria-pressed", "false"),
            (
                "title",
                "Add your signature, or sign digitally with a certificate",
            ),
        ],
    },
    Initial {
        id: "fill-sign-draw",
        text: "Draw signature…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "field-designer",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "field-designer-hint",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "field-new-kind",
        text: "",
        value: "text",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "field-new-name",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "128")],
    },
    Initial {
        id: "field-new-x",
        text: "",
        value: "72",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "field-new-y",
        text: "",
        value: "72",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "field-new-w",
        text: "",
        value: "180",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "field-new-h",
        text: "",
        value: "24",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "field-new-maxlen",
        text: "",
        value: "0",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("min", "0"), ("max", "4096")],
    },
    Initial {
        id: "field-new-label",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "256")],
    },
    Initial {
        id: "field-new-default",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "4096")],
    },
    Initial {
        id: "field-new-add",
        text: "Add field",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "field-prop-name",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "128")],
    },
    Initial {
        id: "field-prop-maxlen",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("min", "0"), ("max", "4096")],
    },
    Initial {
        id: "field-prop-label",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "256")],
    },
    Initial {
        id: "field-prop-default",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "4096")],
    },
    Initial {
        id: "field-prop-apply",
        text: "Apply",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "field-prop-delete",
        text: "Delete",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "panel-links",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "tab-links")],
    },
    Initial {
        id: "links",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "links-heading")],
    },
    Initial {
        id: "links-heading",
        text: "Links on page 1",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "link-page-add",
        text: "Link to page…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            (
                "title",
                "Link the current search match to another page; Cmd/Ctrl+Shift+K",
            ),
            ("aria-keyshortcuts", "Shift+Control+K Shift+Meta+K"),
        ],
    },
    Initial {
        id: "link-uri-add",
        text: "Web link…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("title", "Link the current search match to a web address")],
    },
    Initial {
        id: "link-draw",
        text: "Draw link",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[
            ("aria-pressed", "false"),
            ("title", "Drag on the page to draw a link"),
        ],
    },
    Initial {
        id: "links-empty",
        text: "No links on this page.",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "text-panel",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "tab-text")],
    },
    Initial {
        id: "content-editor",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "content-editor-heading")],
    },
    Initial {
        id: "content-editor-heading",
        text: "Existing content",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-add-arm",
        text: "Pick baseline point",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-pressed", "false")],
    },
    Initial {
        id: "content-add-font",
        text: "",
        value: "Helvetica",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-add-size",
        text: "",
        value: "12",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("min", "4"), ("max", "400")],
    },
    Initial {
        id: "content-add-color",
        text: "",
        value: "#000000",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-add-text",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "4096")],
    },
    Initial {
        id: "content-add-go",
        text: "Add text",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-refresh",
        text: "Refresh",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("title", "Scan this page again for text that can be edited")],
    },
    Initial {
        id: "content-targets",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "content-editor-heading")],
    },
    Initial {
        id: "content-empty",
        text: "No content on this page.",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-edit-form",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-new-text",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("maxlength", "4096"),
            ("aria-labelledby", "content-edit-label"),
        ],
    },
    Initial {
        id: "content-repertoire",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-apply",
        text: "Apply",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-delete",
        text: "Delete",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-object-form",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-rotate-ccw",
        text: "⟲",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Rotate counter-clockwise")],
    },
    Initial {
        id: "content-rotate-cw",
        text: "⟳",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Rotate clockwise")],
    },
    Initial {
        id: "content-flip-h",
        text: "Flip H",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Flip horizontal")],
    },
    Initial {
        id: "content-flip-v",
        text: "Flip V",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Flip vertical")],
    },
    Initial {
        id: "content-crop",
        text: "Crop",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Crop"), ("aria-pressed", "false")],
    },
    Initial {
        id: "content-replace",
        text: "Replace…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-delete-object",
        text: "Delete",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-apply-object",
        text: "Apply",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-cancel-object",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-find",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "256")],
    },
    Initial {
        id: "content-replace-with",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "4096")],
    },
    Initial {
        id: "content-scope-page",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[("aria-label", "Page")],
    },
    Initial {
        id: "content-scope-run",
        text: "",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Selected run only")],
    },
    Initial {
        id: "content-preview-button",
        text: "Preview",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-replace-apply",
        text: "Replace checked",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "content-replace-cancel",
        text: "Clear",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "extracted-heading",
        text: "Extracted text",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "text-empty",
        text: "No text to show yet.",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "panel-layers",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "tab-layers")],
    },
    Initial {
        id: "layers",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "layers-heading")],
    },
    Initial {
        id: "layers-heading",
        text: "Layers",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "layers-note",
        text: "View only; a layer toggle is not saved.",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "layers-empty",
        text: "This document has no optional-content layers.",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "status",
        text: "Starting…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "page-status",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "fields-status",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "nav-status",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "edit-status",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "zoom-bar",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Zoom")],
    },
    Initial {
        id: "zoom-out",
        text: "Zoom out",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("aria-label", "Zoom out"),
            ("aria-keyshortcuts", "Control+- Meta+-"),
        ],
    },
    Initial {
        id: "zoom-in",
        text: "Zoom in",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("aria-label", "Zoom in"),
            ("aria-keyshortcuts", "Control+= Meta+="),
        ],
    },
    Initial {
        id: "fit-width",
        text: "Fit width",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[("aria-pressed", "true")],
    },
    Initial {
        id: "fit-page",
        text: "Fit page",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-pressed", "false")],
    },
    Initial {
        id: "zoom-100",
        text: "100%",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "zoom-level",
        text: "100%",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "verify-open",
        text: "Verify",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "note-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "note-dialog-heading")],
    },
    Initial {
        id: "note-dialog-heading",
        text: "Edit note",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "note-text",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "4096")],
    },
    Initial {
        id: "note-save",
        text: "Save",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "note-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "help-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "help-heading")],
    },
    Initial {
        id: "help-heading",
        text: "What can be edited",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "help-close",
        text: "Close",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("aria-labelledby", "redact-heading"),
            ("aria-describedby", "redact-explain"),
        ],
    },
    Initial {
        id: "redact-heading",
        text: "Permanently remove everything inside 0 boxes?",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-explain",
        text: "Text, pictures, drawings, comments and form fields inside the boxes are deleted from the file, not just covered. The result is saved as a new file; your original is not changed.",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-hidden",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "redact-sanitize-info",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "redact-sanitize-xmp",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "redact-sanitize-attachments",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "redact-sanitize-outlines",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "redact-sanitize-annots",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "redact-sanitize-links",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "redact-sanitize-fields",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "redact-sanitize-layers",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "redact-sanitize-struct",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "redact-limit",
        text: "Redaction works on files up to 12.5 MB in this version.",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-apply",
        text: "Redact and save as…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-find-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "redact-find-heading")],
    },
    Initial {
        id: "redact-find-heading",
        text: "Find text to redact",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-search-text",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "256")],
    },
    Initial {
        id: "redact-search-go",
        text: "Add boxes",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-find-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-pending-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("aria-labelledby", "redact-pending-heading"),
            ("aria-describedby", "redact-pending-text"),
        ],
    },
    Initial {
        id: "redact-pending-heading",
        text: "Areas are marked but not redacted yet.",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-pending-text",
        text: "Saving now keeps the boxes as marks. Everything under them stays in the file until you redact it.",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-pending-now",
        text: "Redact now…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-pending-marks",
        text: "Save marks only",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-pending-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-ask-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[
            ("aria-labelledby", "redact-ask-heading"),
            ("aria-describedby", "redact-ask-text"),
        ],
    },
    Initial {
        id: "redact-ask-heading",
        text: "Redact?",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-ask-yes",
        text: "Continue",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-ask-alt",
        text: "Continue",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "redact-ask-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "stamp-icon-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "stamp-icon-heading")],
    },
    Initial {
        id: "stamp-icon-heading",
        text: "Stamp",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "stamp-icon-save",
        text: "Add",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "stamp-icon-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "sigpad-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "sigpad-heading")],
    },
    Initial {
        id: "sigpad-heading",
        text: "Your signature",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "sigpad-canvas",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Signature drawing area")],
    },
    Initial {
        id: "sigpad-status",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "sigpad-clear",
        text: "Clear",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "sigpad-image",
        text: "Use an image…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "sigpad-remove",
        text: "Remove saved signature",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "sigpad-save",
        text: "Save",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "sigpad-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "stamp-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "stamp-dialog-heading")],
    },
    Initial {
        id: "stamp-dialog-heading",
        text: "Watermark",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "stamp-kind",
        text: "",
        value: "watermark",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "stamp-text",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "200")],
    },
    Initial {
        id: "stamp-align",
        text: "",
        value: "left",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "stamp-hint",
        text: "Placeholders: {page} {pages} {label} {bates}",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "stamp-save",
        text: "Apply",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "stamp-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "labels-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "labels-dialog-heading")],
    },
    Initial {
        id: "labels-dialog-heading",
        text: "Page labels",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "labels-from",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("min", "1")],
    },
    Initial {
        id: "labels-style",
        text: "",
        value: "D",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "labels-prefix",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "32")],
    },
    Initial {
        id: "labels-start",
        text: "",
        value: "1",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("min", "1")],
    },
    Initial {
        id: "labels-save",
        text: "Set labels",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "labels-clear",
        text: "Clear from this page",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "labels-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "export-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "export-dialog-heading")],
    },
    Initial {
        id: "export-dialog-heading",
        text: "Export",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "export-kind",
        text: "",
        value: "png",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "export-dpi",
        text: "",
        value: "150",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "export-start",
        text: "Export",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "export-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "insert-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "insert-dialog-heading")],
    },
    Initial {
        id: "insert-dialog-heading",
        text: "Insert pages",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "insert-after",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("min", "0")],
    },
    Initial {
        id: "insert-start",
        text: "Insert",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "insert-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "extract-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "extract-dialog-heading")],
    },
    Initial {
        id: "extract-dialog-heading",
        text: "Extract pages",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "extract-start",
        text: "Extract",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "extract-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "bookmark-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "bookmark-dialog-heading")],
    },
    Initial {
        id: "bookmark-dialog-heading",
        text: "Add bookmark",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "bookmark-title",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "4096")],
    },
    Initial {
        id: "bookmark-save",
        text: "Save",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "bookmark-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "link-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "link-dialog-heading")],
    },
    Initial {
        id: "link-dialog-heading",
        text: "Link to page",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "link-kind",
        text: "",
        value: "page",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "link-target-label",
        text: "Page number",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "link-target",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "2048")],
    },
    Initial {
        id: "link-save",
        text: "Save",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "link-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "print-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Print")],
    },
    Initial {
        id: "print-status",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "print-start",
        text: "Print",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "print-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "verify-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "verify-heading")],
    },
    Initial {
        id: "verify-heading",
        text: "Verify signatures",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "verify-summary",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "verify-trust-add",
        text: "Trust certificates…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "verify-trust-clear",
        text: "Forget all trusted certificates",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "verify-refresh",
        text: "Refresh",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "verify-close",
        text: "Close",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "batch-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "batch-heading")],
    },
    Initial {
        id: "batch-heading",
        text: "Batch",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "batch-pick",
        text: "Choose folder…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "batch-kind",
        text: "",
        value: "footer",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "batch-align",
        text: "",
        value: "left",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "batch-text",
        text: "",
        value: "{bates}",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "200")],
    },
    Initial {
        id: "batch-summary",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "batch-list",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-label", "Batch files")],
    },
    Initial {
        id: "batch-start",
        text: "Start",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "batch-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "batch-close",
        text: "Close",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "sign-choice-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "sign-choice-heading")],
    },
    Initial {
        id: "sign-choice-heading",
        text: "Sign this document",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "sign-choice-picture",
        text: "Add my signatureDraw it or use a picture of it, then click where it goes. Quick, but it does not prove who signed.",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "sign-choice-certificate",
        text: "Sign digitally with a certificateUses your certificate file. Anyone can check who signed and that nothing changed since. Saves a signed copy, which then opens.",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "sign-choice-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "cert-sign-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "cert-sign-heading")],
    },
    Initial {
        id: "cert-sign-heading",
        text: "Sign with certificate",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "cert-sign-pick",
        text: "Choose certificate…",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "cert-sign-file",
        text: "No certificate selected",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "cert-sign-password",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "1024")],
    },
    Initial {
        id: "cert-sign-field",
        text: "",
        value: "",
        hidden: true,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "cert-sign-page",
        text: "",
        value: "1",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("min", "1")],
    },
    Initial {
        id: "cert-sign-visible",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "cert-sign-x",
        text: "",
        value: "376",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "cert-sign-y",
        text: "",
        value: "696",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "cert-sign-w",
        text: "",
        value: "200",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "cert-sign-h",
        text: "",
        value: "60",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "cert-sign-reason",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "200")],
    },
    Initial {
        id: "cert-sign-status",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-live", "polite")],
    },
    Initial {
        id: "cert-sign-go",
        text: "Sign and save as…",
        value: "",
        hidden: false,
        disabled: true,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "cert-sign-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "security-dialog",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-labelledby", "security-heading")],
    },
    Initial {
        id: "security-heading",
        text: "Security",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "security-current",
        text: "Currently: no encryption",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "security-mode-none",
        text: "",
        value: "none",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "security-mode-aes",
        text: "",
        value: "aes-256",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "security-user",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "1024")],
    },
    Initial {
        id: "security-owner",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("maxlength", "1024")],
    },
    Initial {
        id: "security-perm-print",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "security-perm-copy",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "security-perm-edit",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "security-perm-annotate",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "security-perm-form",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: true,
        attrs: &[],
    },
    Initial {
        id: "security-apply",
        text: "Apply",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "security-cancel",
        text: "Cancel",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[],
    },
    Initial {
        id: "print-pages",
        text: "",
        value: "",
        hidden: false,
        disabled: false,
        open: false,
        checked: false,
        attrs: &[("aria-hidden", "true")],
    },
];

/// Sets every element's initial state as index.html writes it (before any script runs).
pub fn init_dom(dom: &mut Dom) {
    for e in INITIAL {
        dom.init(
            e.id,
            Node {
                text: e.text.to_string(),
                value: e.value.to_string(),
                hidden: e.hidden,
                disabled: e.disabled,
                open: e.open,
                checked: e.checked,
                attrs: e
                    .attrs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state() {
        let mut dom = Dom::new();
        init_dom(&mut dom);
        assert!(dom.disabled(SAVE_BUTTON));
        assert!(dom.hidden(DIRTY_CHIP_EL));
        assert_eq!(dom.text(DOCUMENT_TITLE_EL), "Tandem PDF");
        assert_eq!(dom.value(PAGE_INPUT), "1");
        assert_eq!(STATUS_EL, "status");
        assert_eq!(NAMES.len(), 369);
    }
}
