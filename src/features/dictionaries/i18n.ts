import type { AppError, DictionaryKind, DictionaryLayer, SourceLanguage, UiLocale } from '../../lib/types';

const vi = {
  title: 'Từ điển', entries: 'Tra cứu / biên tập', imports: 'Nhập / nạp lại', shortcuts: 'Gõ tắt', attribution: 'Nguồn dữ liệu / giấy phép',
  close: 'Đóng', cancel: 'Hủy', save: 'Lưu', add: 'Thêm mục', edit: 'Biên tập', delete: 'Xóa mục', confirmDelete: 'Xóa mục này khỏi từ điển đang chọn?',
  deleteNotice: 'Mục đã xóa sẽ không xuất hiện lại khi nạp lại bản nhập. Lịch sử vẫn được giữ.', discardEdits: 'Bỏ các thay đổi chưa lưu?', discard: 'Bỏ thay đổi',
  nativeUnavailable: 'Từ điển, tệp và giấy phép chỉ khả dụng trong ứng dụng máy tính. Chế độ trình duyệt không dùng dữ liệu mô phỏng.',
  loading: 'Đang nạp…', working: 'Đang xử lý…', refresh: 'Làm mới', language: 'Ngôn ngữ', zh: 'Tiếng Trung', ja: 'Tiếng Nhật', kind: 'Loại từ điển', allKinds: 'Tất cả các loại',
  dictionary: 'Từ điển', allDictionaries: 'Tất cả từ điển', bundled: 'Dữ liệu kèm ứng dụng', imported: 'Dữ liệu đã nhập', edited: 'Biên tập cá nhân',
  aliasesCount: 'Cách viết khác', editsCount: 'Mục đã biên tập', tombstonesCount: 'Mục đã xóa', payload: 'Nội dung phụ trợ (văn bản)', format: 'Định dạng', legacyFormat: 'Đầu mục=nghĩa', cedictFormat: 'CEDict', ignoredFormat: 'Mỗi dòng một cụm bỏ qua',
  repair: 'Sao lưu và tạo cơ sở dữ liệu mới…', repairTitle: 'Dữ liệu người dùng chưa sẵn sàng', repairNotice: 'Tệp hiện tại không bị xóa. Chỉ khi xác nhận, ứng dụng sao chép tệp cùng WAL/SHM sang đích sao lưu chưa có, rồi tạo cơ sở dữ liệu mới riêng. Cơ sở dữ liệu mới không có bản nhập hoặc biên tập cũ; bản sao vẫn còn để khôi phục. Có thể nhập lại từ điển gốc.',
  confirmRepair: 'Xác nhận sao lưu và tạo mới', repairSuccess: 'Đã giữ bản sao dữ liệu cũ và chuyển sang cơ sở dữ liệu mới.', backupExists: 'Đích sao lưu đã tồn tại; chọn đích mới.', dictionaryFiles: 'Tệp từ điển', allFiles: 'Tất cả tệp', chooseBackup: 'Chọn đích sao lưu mới', originalFiles: 'Tệp nhập gốc', autoDetected: 'tự nhận diện', explicitEncoding: 'mã hóa được chọn',
  search: 'Tìm', searchLabel: 'Tìm đầu mục', searchHint: 'Đầu mục hoặc cách viết khác', noResults: 'Không có mục phù hợp. Có thể thêm mục chưa có.', more: 'Trang tiếp', previous: 'Trang trước',
  headword: 'Đầu mục', meanings: 'Nghĩa tiếng Việt (mỗi dòng một nghĩa, giữ nguyên thứ tự)', reading: 'Cách đọc (không bắt buộc)', pos: 'Từ loại (không bắt buộc)', aliases: 'Cách viết khác (mỗi dòng một mục)',
  sourceUrls: 'Địa chỉ nguồn (mỗi dòng một URL)', provenance: 'Nguồn / lớp dữ liệu', coverage: 'Phạm vi dữ liệu', activeEntries: 'Đầu mục đang hoạt động', revision: 'Phiên bản từ điển',
  noSelection: 'Chọn một mục để xem nghĩa, cách đọc, nguồn và lịch sử; hoặc thêm đầu mục mới.', overridesNotice: 'Biên tập được lưu trong SQLite riêng. Dữ liệu gốc kèm ứng dụng không bị sửa. Nạp lại vẫn giữ các biên tập này.',
  ignoredHint: 'Cụm bỏ qua không cần nghĩa; văn bản nguồn không bị xóa.', ruleHint: 'Luật Nhân dùng {0} làm cụm bắt. Biểu thức không hợp lệ sẽ được báo trước khi lưu hoặc nhập.',
  hanHint: 'Hán Việt chỉ nhận một ký tự Han. Cách đọc đầu tiên được dùng cho kết quả.', history: 'Lịch sử mục', noHistory: 'Chưa có lịch sử biên tập.', before: 'Trước', after: 'Sau', operation: 'Thao tác', deleted: 'Đã xóa',
  chooseDictionary: 'Chọn từ điển lưu mục', newDictionary: 'Từ điển cá nhân mới', dictionaryName: 'Tên từ điển', export: 'Xuất từ điển…', exported: 'Đã xuất UTF-8 đến đích đã chọn.',
  exportNotice: 'Chọn đích xuất riêng; ứng dụng không tự ghi đè tệp nhập gốc. Các cách đọc Nhật tùy chọn vẫn lưu trong cơ sở dữ liệu; văn bản key=value chỉ xuất nghĩa.',
  chooseFile: 'Chọn tệp từ điển…', chooseConfig: 'Chọn Dictionaries.config…', remap: 'Chọn tệp thay thế…', reload: 'Xem trước nạp lại', import: 'Nhập các mục đã chọn', preview: 'Xem trước',
  importInstructions: 'Chọn tệp riêng hoặc cấu hình cũ. Kiểm tra mã hóa và các dòng lỗi trước khi nhập. Mỗi lần nhập thay thế lớp nhập của từ điển đó, không xóa biên tập cá nhân.',
  sourceFile: 'Tệp nguồn', encoding: 'Mã hóa', automatic: 'Tự nhận diện / BOM', detectedEncoding: 'Mã hóa đã dùng', decodingFailed: 'Không thể giải mã chính xác; chọn mã hóa khác rồi xem trước lại.',
  accepted: 'Hợp lệ', duplicates: 'Trùng (giữ mục đầu)', malformed: 'Không hợp lệ', line: 'Dòng', code: 'Loại lỗi', rawPreview: 'Văn bản sau giải mã', previewEntries: 'Mẫu mục nhập', diagnostics: 'Các dòng cần kiểm tra',
  noDiagnostics: 'Không có dòng trùng hoặc lỗi.', emptyPreview: 'Không có mục hợp lệ để nhập.', previewRequired: 'Xem trước trước khi xác nhận nhập.', importedNotice: 'Đã nhập và công bố phiên bản từ điển mới cho mọi cửa sổ.',
  selectResolved: 'Chọn các tệp đã tìm thấy muốn nhập. Mục chưa giải quyết là tùy chọn; chọn tệp thay thế hoặc bỏ chọn.', unresolved: 'Không tìm thấy / đường dẫn cần thay thế', resolved: 'Đã tìm thấy', selected: 'Chọn',
  configKey: 'Khóa cấu hình', path: 'Đường dẫn', reloadInstructions: 'Chọn một từ điển đã nhập bên dưới để xem trước lại tệp nguồn. Không nạp lại âm thầm.', noReloadable: 'Chưa có từ điển nhập nào có tệp nguồn để nạp lại.',
  ruleMode: 'Thuật toán Nhân', mode1: '1 — Đại từ', mode2: '2 — Đại từ + tên', mode3: '3 — Đại từ + tên + VietPhrase',
  importShortcuts: 'Nhập Shortcuts.txt…', exportShortcuts: 'Xuất Shortcuts.txt…', shortcutKey: 'Từ gõ tắt', shortcutValue: 'Văn bản thay thế', shortcutNotice: 'Khóa được chuyển thành chữ thường, mục trùng đầu tiên được giữ. Xuất theo độ dài khóa giảm dần, rồi thứ tự khóa.',
  noShortcuts: 'Chưa có mục gõ tắt.', shortcutSaved: 'Đã lưu gõ tắt.', shortcutDeleted: 'Đã xóa gõ tắt.', dataNotice: 'Cách đọc và nghĩa ngoại tuyến là hỗ trợ tra cứu, không phải bản dịch máy hoàn chỉnh. Dữ liệu người dùng nhập luôn lưu cục bộ, không đưa vào bản phát hành.',
  license: 'Giấy phép', resource: 'Tài nguyên', manifest: 'Bản kê tài nguyên', source: 'Nguồn', saved: 'Đã lưu mục và cập nhật mọi cửa sổ.',
  invalidEntry: 'Kiểm tra đầu mục, nghĩa và loại từ điển. Các mục bỏ qua không cần nghĩa; Hán Việt cần một ký tự Han.',
  unknownDiagnostic: 'Dòng cần kiểm tra', malformedRecord: 'Dòng không đúng định dạng', emptyKey: 'Đầu mục trống', duplicateRecord: 'Đầu mục trùng; giữ mục đầu', invalidRule: 'Biểu thức Luật Nhân không hợp lệ',
  missingPlaceholder: 'Luật Nhân phải có đúng một {0}', invalidEncoding: 'Mã hóa không hợp lệ', invalidShortcut: 'Gõ tắt cần khóa và văn bản thay thế không trống',
  emptyMeaning: 'Nghĩa trống', invalidHanCharacter: 'Hán Việt cần một ký tự Han', invalidCedict: 'Dòng CEDict không hợp lệ', unknownConfigKey: 'Khóa cấu hình không được nhận diện', invalidRuleAlgorithm: 'Thuật toán Nhân chỉ nhận 1, 2 hoặc 3', missingPath: 'Đường dẫn tệp trống',
  meaningsPreview: 'Nghĩa / nội dung gốc', meaningsText: 'Nghĩa văn bản gốc (mỗi dòng một nghĩa, giữ thứ tự)', hanMeanings: 'Các cách đọc Hán Việt (mỗi dòng một cách đọc, dùng mục đầu tiên)', lookupOnlyNotice: 'Từ điển phụ trợ chỉ để tra nghĩa. Nội dung giữ nguyên ngôn ngữ gốc, không được gọi là bản dịch tiếng Việt.',
  failure: 'Không thể hoàn tất thao tác từ điển.', databaseError: 'Không thể truy cập cơ sở dữ liệu từ điển. Dữ liệu được giữ nguyên; kiểm tra tệp hoặc khôi phục từ bản sao lưu.',
  exportNotRepresentable: 'Một số mục chứa dấu = hoặc xuống dòng không thể xuất nguyên vẹn sang định dạng cũ một dòng key=value. Dữ liệu vẫn được giữ trong cơ sở dữ liệu gốc; không có bản xuất bị cắt bỏ nội dung.',
  saveFailure: 'Không thể ghi tệp tại đích đã chọn. Kiểm tra quyền ghi và dung lượng ổ đĩa, rồi chọn đích và xuất lại.',
  missingFile: 'Không tìm thấy tệp. Chọn tệp thay thế để tiếp tục.', stalePreview: 'Bản xem trước không còn khớp dữ liệu hiện tại. Xem trước lại rồi xác nhận.', resourceError: 'Không thể đọc tài nguyên từ điển kèm ứng dụng.',
  invalidConfig: 'Không thể đọc cấu hình từ điển. Kiểm tra định dạng và mã hóa.', entryMissing: 'Mục từ điển không còn tồn tại. Làm mới kết quả.', originalDestination: 'Hãy chọn đích khác tệp nhập gốc.',
  importAction: 'Nhập', updateAction: 'Biên tập', deleteAction: 'Xóa', createAction: 'Thêm', results: 'Kết quả', total: 'Tổng',
} as const;

export type DictionaryText = keyof typeof vi;
const en: Record<DictionaryText, string> = {
  title: 'Dictionaries', entries: 'Search / edit', imports: 'Import / reload', shortcuts: 'Text shortcuts', attribution: 'Data sources / licenses',
  close: 'Close', cancel: 'Cancel', save: 'Save', add: 'Add entry', edit: 'Edit', delete: 'Delete entry', confirmDelete: 'Delete this entry from the selected dictionary?',
  deleteNotice: 'Deleted entries will not return when imported data is reloaded. History is retained.', discardEdits: 'Discard unsaved changes?', discard: 'Discard changes',
  nativeUnavailable: 'Dictionaries, files and licenses are available only in the desktop application. Browser mode has no simulated data.',
  loading: 'Loading…', working: 'Working…', refresh: 'Refresh', language: 'Language', zh: 'Chinese', ja: 'Japanese', kind: 'Dictionary kind', allKinds: 'All kinds',
  dictionary: 'Dictionary', allDictionaries: 'All dictionaries', bundled: 'Bundled data', imported: 'Imported data', edited: 'Local edit',
  aliasesCount: 'Aliases', editsCount: 'Edited entries', tombstonesCount: 'Deleted entries', payload: 'Auxiliary payload (text)', format: 'Format', legacyFormat: 'Headword=meanings', cedictFormat: 'CEDict', ignoredFormat: 'One ignored phrase per line',
  repair: 'Back up and create a new database…', repairTitle: 'User data is unavailable', repairNotice: 'The existing file is not deleted. Only after confirmation, the app copies it and its WAL/SHM to an unused backup destination, then creates a separate new database. The new database has no previous imports or edits; the backup remains for recovery. Original dictionaries can be reimported.',
  confirmRepair: 'Confirm backup and new database', repairSuccess: 'Previous data backed up and a new database selected.', backupExists: 'The backup destination already exists; choose a new destination.', dictionaryFiles: 'Dictionary files', allFiles: 'All files', chooseBackup: 'Choose a new backup destination', originalFiles: 'Original imports', autoDetected: 'automatically detected', explicitEncoding: 'selected encoding',
  search: 'Search', searchLabel: 'Search headwords', searchHint: 'Headword or alias', noResults: 'No matching entry. You can add an unknown headword.', more: 'Next page', previous: 'Previous page',
  headword: 'Headword', meanings: 'Vietnamese meanings (one per line, in order)', reading: 'Reading (optional)', pos: 'Part of speech (optional)', aliases: 'Aliases (one per line)',
  sourceUrls: 'Source URLs (one per line)', provenance: 'Provenance / data layer', coverage: 'Data coverage', activeEntries: 'Active headwords', revision: 'Dictionary revision',
  noSelection: 'Choose an entry to inspect meanings, reading, provenance and history, or add a new headword.', overridesNotice: 'Edits are saved separately in SQLite. Bundled originals are not changed. Reloading preserves these edits.',
  ignoredHint: 'Ignored phrases need no meanings; source text is not deleted.', ruleHint: 'Luật Nhân uses {0} as a captured phrase. Invalid expressions are reported before save or import.',
  hanHint: 'Hán Việt accepts one Han character. The first reading is used in generated output.', history: 'Entry history', noHistory: 'No local edit history yet.', before: 'Before', after: 'After', operation: 'Operation', deleted: 'Deleted',
  chooseDictionary: 'Choose the entry destination', newDictionary: 'New local dictionary', dictionaryName: 'Dictionary name', export: 'Export dictionary…', exported: 'UTF-8 export saved to the chosen destination.',
  exportNotice: 'Choose an explicit export destination; original imports are never overwritten automatically. Optional Japanese readings remain in the database; key=value text exports meanings only.',
  chooseFile: 'Choose dictionary file…', chooseConfig: 'Choose Dictionaries.config…', remap: 'Choose replacement file…', reload: 'Preview reload', import: 'Import selected items', preview: 'Preview',
  importInstructions: 'Choose an individual file or legacy configuration. Review encoding and malformed lines before import. Each import replaces that dictionary’s imported layer while preserving local edits.',
  sourceFile: 'Source file', encoding: 'Encoding', automatic: 'Detect automatically / BOM', detectedEncoding: 'Encoding used', decodingFailed: 'Decoding failed without replacing invalid bytes; choose another encoding and preview again.',
  accepted: 'Accepted', duplicates: 'Duplicates (first retained)', malformed: 'Malformed', line: 'Line', code: 'Diagnostic', rawPreview: 'Decoded text preview', previewEntries: 'Sample imported entries', diagnostics: 'Lines to review',
  noDiagnostics: 'No duplicate or malformed lines.', emptyPreview: 'No valid entries to import.', previewRequired: 'Preview before confirming import.', importedNotice: 'Imported and published a new dictionary revision to every window.',
  selectResolved: 'Select the resolved files to import. Unresolved items are optional; remap them or leave them unselected.', unresolved: 'Missing / path needs remapping', resolved: 'Resolved', selected: 'Select',
  configKey: 'Configuration key', path: 'Path', reloadInstructions: 'Choose a previously imported dictionary below to preview its source again. Reload never imports silently.', noReloadable: 'No imported dictionary has a source file available for reload.',
  ruleMode: 'Luật Nhân algorithm', mode1: '1 — Pronouns', mode2: '2 — Pronouns + names', mode3: '3 — Pronouns + names + VietPhrase',
  importShortcuts: 'Import Shortcuts.txt…', exportShortcuts: 'Export Shortcuts.txt…', shortcutKey: 'Shortcut key', shortcutValue: 'Replacement text', shortcutNotice: 'Keys are lowercased; the first duplicate is retained. Exports are sorted by descending key length, then key.',
  noShortcuts: 'No text shortcuts yet.', shortcutSaved: 'Text shortcut saved.', shortcutDeleted: 'Text shortcut deleted.', dataNotice: 'Offline readings and glosses assist dictionary lookup; they are not fluent machine translation. User-imported corpora remain local and are never included in releases.',
  license: 'License', resource: 'Resource', manifest: 'Resource manifest', source: 'Source', saved: 'Entry saved and every window invalidated.',
  invalidEntry: 'Check the headword, meanings and dictionary kind. Ignored entries need no meanings; Hán Việt needs one Han character.',
  unknownDiagnostic: 'Line needs review', malformedRecord: 'Malformed record', emptyKey: 'Empty headword', duplicateRecord: 'Duplicate headword; first retained', invalidRule: 'Invalid Luật Nhân expression',
  missingPlaceholder: 'Luật Nhân requires exactly one {0}', invalidEncoding: 'Invalid encoding', invalidShortcut: 'A shortcut needs a nonempty key and replacement',
  emptyMeaning: 'Empty meaning', invalidHanCharacter: 'Hán Việt needs one Han character', invalidCedict: 'Invalid CEDict record', unknownConfigKey: 'Unrecognized configuration key', invalidRuleAlgorithm: 'Luật Nhân algorithm must be 1, 2 or 3', missingPath: 'Empty file path',
  meaningsPreview: 'Meanings / original payload', meaningsText: 'Original text meanings (one per line, in order)', hanMeanings: 'Hán Việt readings (one per line, first used)', lookupOnlyNotice: 'Auxiliary dictionaries are lookup-only. Text retains its original language and is not presented as a Vietnamese translation.',
  failure: 'The dictionary operation could not be completed.', databaseError: 'The dictionary database could not be accessed. Data is preserved; inspect the file or restore a backup.',
  exportNotRepresentable: 'Some entries contain = or line breaks that cannot be represented faithfully in the legacy one-line key=value format. Data remains in the native database; no truncated export was written.',
  saveFailure: 'The file could not be written at the chosen destination. Check write permissions and disk space, then choose a destination and export again.',
  missingFile: 'File not found. Choose a replacement to continue.', stalePreview: 'The preview no longer matches current data. Preview again before confirming.', resourceError: 'Bundled dictionary resources could not be read.',
  invalidConfig: 'The dictionary configuration could not be read. Check its format and encoding.', entryMissing: 'The dictionary entry no longer exists. Refresh the results.', originalDestination: 'Choose a destination other than the original import file.',
  importAction: 'Import', updateAction: 'Edit', deleteAction: 'Delete', createAction: 'Add', results: 'Results', total: 'Total',
};

export function dt(locale: UiLocale, key: DictionaryText): string { return (locale === 'vi' ? vi : en)[key]; }

const kindNames: Record<DictionaryKind, [string, string]> = {
  primaryNames: ['Tên chính (Names)', 'Primary names (Names)'], secondaryNames: ['Tên phụ (NamesPhu)', 'Secondary names (NamesPhu)'],
  vietPhrase: ['VietPhrase', 'VietPhrase'], hanViet: ['Hán Việt', 'Hán Việt readings'], japanese: ['Từ Nhật → Việt', 'Japanese → Vietnamese'],
  pronouns: ['Đại từ (Pronouns)', 'Pronouns'], rules: ['Luật Nhân', 'Luật Nhân rules'], ignored: ['Cụm Trung bỏ qua', 'Ignored Chinese phrases'],
  cedict: ['CEDict (tra nghĩa)', 'CEDict (lookup only)'], babylon: ['Babylon (tra nghĩa)', 'Babylon (lookup only)'],
  lacViet: ['Lạc Việt (tra nghĩa)', 'Lạc Việt (lookup only)'], thieuChuu: ['Thiều Chửu (tra nghĩa)', 'Thiều Chửu (lookup only)'], auxiliary: ['Phụ trợ (tra nghĩa)', 'Auxiliary (lookup only)'],
};
export const DICTIONARY_KINDS = Object.keys(kindNames) as DictionaryKind[];
export function kindLabel(locale: UiLocale, kind: DictionaryKind): string { return kindNames[kind][locale === 'vi' ? 0 : 1]; }
export function languageLabel(locale: UiLocale, language: SourceLanguage): string { return dt(locale, language); }
export function layerLabel(locale: UiLocale, layer: DictionaryLayer): string { return dt(locale, layer); }

export function dictionaryError(locale: UiLocale, error: AppError): string {
  const code = error.code.toLowerCase().replaceAll('_', '');
  let key: DictionaryText = 'failure';
  if (code.includes('nativeunavailable')) key = 'nativeUnavailable';
  else if (code.includes('backupexists')) key = 'backupExists';
  else if (code.includes('exportnotrepresentable')) key = 'exportNotRepresentable';
  else if (code === 'savefailed') key = 'saveFailure';
  else if (code.includes('destination')) key = 'originalDestination';
  else if (code === 'dictionarynotfound') key = 'entryMissing';
  else if (code.includes('encoding') || code.includes('decode')) key = 'decodingFailed';
  else if (code.includes('database') || code.includes('sqlite') || code.includes('storage')) key = 'databaseError';
  else if (code.includes('preview') || code.includes('stale')) key = 'stalePreview';
  else if (code.includes('resource') || code.includes('bundled')) key = 'resourceError';
  else if (code.includes('config')) key = 'invalidConfig';
  else if (code.includes('notfound') || code.includes('file') || code.includes('path')) key = 'missingFile';
  else if (code.includes('rule') || code.includes('regex')) key = 'invalidRule';
  else if (code.includes('shortcut')) key = 'invalidShortcut';
  else if (code.includes('entry') || code.includes('dictionarykind') || code.includes('validation')) key = 'invalidEntry';
  return `${dt(locale, key)} (${error.code})`;
}

export function diagnosticLabel(locale: UiLocale, code: string): string {
  const normalized = code.toLowerCase().replaceAll('_', '');
  if (normalized.includes('duplicate')) return dt(locale, 'duplicateRecord');
  if (normalized.includes('emptykey') || normalized.includes('emptyheadword')) return dt(locale, 'emptyKey');
  if (normalized.includes('emptymeaning')) return dt(locale, 'emptyMeaning');
  if (normalized.includes('hancharacter')) return dt(locale, 'invalidHanCharacter');
  if (normalized.includes('cedict')) return dt(locale, 'invalidCedict');
  if (normalized.includes('configkey')) return dt(locale, 'unknownConfigKey');
  if (normalized.includes('rulealgorithm')) return dt(locale, 'invalidRuleAlgorithm');
  if (normalized.includes('missingpath')) return dt(locale, 'missingPath');
  if (normalized.includes('placeholder')) return dt(locale, 'missingPlaceholder');
  if (normalized.includes('rule') || normalized.includes('regex')) return dt(locale, 'invalidRule');
  if (normalized.includes('encoding') || normalized.includes('decode')) return dt(locale, 'invalidEncoding');
  if (normalized.includes('malformed') || normalized.includes('separator') || normalized.includes('record')) return dt(locale, 'malformedRecord');
  return dt(locale, 'unknownDiagnostic');
}
