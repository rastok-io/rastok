set nocompatible
set nomore
let s:root = fnamemodify(expand('<sfile>:p'), ':h:h:h')
execute 'set runtimepath^=' . fnameescape(s:root . '/editors/vim')
filetype plugin on
syntax enable
edit test.rkg
call assert_equal('rastok', &filetype)
call assert_equal('rastok', b:current_syntax)
call assert_equal('// %s', &commentstring)
call assert_equal('s1:/*,mb:*,ex:*/,://', &comments)
let s:checks = 4

function! s:Buffer(lines) abort
  silent %delete _
  call setline(1, a:lines)
  syntax sync fromstart
endfunction

function! s:Group(line, text, group) abort
  " Traverse the edited first line before querying a later cached line.
  call synID(1, 1, 1)
  let column = stridx(getline(a:line), a:text) + 1
  call assert_true(column > 0, 'missing test token: ' . a:text)
  call assert_equal(a:group, synIDattr(synID(a:line, column, 1), 'name'), getline(a:line) . ': ' . a:text)
  let s:checks += 1
endfunction

call s:Buffer(['lab smoke {', 'scenario roundtrip {', 'expect 3 app.out == "hello"', 'expect app.failed == nothing', 'simulation alternate : App {}'])
call s:Group(1, 'lab', 'rastokDeclaration')
call s:Group(1, 'smoke', 'rastokName')
call s:Group(2, 'roundtrip', 'rastokName')
call s:Group(3, 'expect', 'rastokKeyword')
call s:Group(3, '3', 'rastokNumber')
call s:Group(3, '==', 'rastokOperator')
call s:Group(3, 'hello', 'rastokString')
call s:Group(4, 'nothing', 'rastokKeyword')
call s:Group(5, 'App', 'rastokExtension')

for word in ['struct', 'union', 'enum', 'map', 'array', 'table', 'bool', 'int', 'long', 'uint', 'float', 'double', 'number', 'decimal', 'text', 'bytes', 'variant', 'date', 'time', 'uuid', 'timestamp', 'timestamp_ntz', 'duration', 'media_type']
  call s:Buffer(['input request: ' . word])
  call s:Group(1, word, 'rastokType')
endfor
call s:Buffer(['val text = decimal("1.25", 18, 2)', 'input price: decimal(18, 6)', 'on start { out = 42u }', 'type note = @struct { n: int @measure(id = n) }', 'model.instance.stop', 'Loop.again', 'Retry.on = [ServerError]', 'connector remote : HttpClient + Retry {}'])
call s:Group(1, 'text', 'rastokName')
call s:Group(1, 'decimal', 'rastokFunction')
call s:Group(2, 'decimal', 'rastokType')
call s:Group(3, 'start', 'rastokKeyword')
call s:Group(3, '42u', 'rastokNumber')
call s:Group(4, '@struct', 'rastokType')
call s:Group(4, '@measure', 'rastokAnnotation')
call s:Group(5, 'model.instance.stop', 'rastokManager')
call s:Group(6, 'Loop.again', 'rastokManager')
call s:Group(7, 'Retry.on', 'rastokManager')
call s:Group(8, 'HttpClient', 'rastokConnector')

for comment in ['/* a // not a line comment */', '/* a /* not a nested comment */']
  call s:Buffer([comment . ' operator recovered', 'operator next'])
  call s:Group(1, 'not', 'rastokComment')
  call s:Group(1, 'recovered', 'rastokName')
  call s:Group(2, 'next', 'rastokName')
endfor
for quote in ['"', "'"]
  for suffix in ['', '\']
    call s:Buffer(['val broken = ' . quote . 'unterminated' . suffix, 'operator recovered'])
    call s:Group(1, 'unterminated', 'rastokString')
    call s:Group(2, 'recovered', 'rastokName')
  endfor
  call s:Buffer(['val value = ' . quote . 'Loop.again // data' . quote, 'operator recovered'])
  call s:Group(1, 'Loop.again', 'rastokString')
  call s:Group(2, 'recovered', 'rastokName')
endfor

call s:Buffer(['val raw = text! {', 'on in { Loop.done "{ nested }" } // raw json!', '}', 'val bytes = base64!("text/plain") { YQ== }', 'operator recovered'])
call s:Group(1, 'text!', 'rastokReader')
call s:Group(2, 'on', 'rastokRawString')
call s:Group(2, 'Loop.done', 'rastokRawBrace')
call s:Group(2, 'nested', 'rastokRawBrace')
call s:Group(2, '// raw', 'rastokRawString')
call s:Group(2, 'json!', 'rastokRawString')
call s:Group(4, 'YQ==', 'rastokRawString')
call s:Group(5, 'recovered', 'rastokName')

call s:Buffer(['val value = json! { "text": "} Loop.again // not DSL", "nested": [true, {"n": 2}] }', 'val scalar = json! "} stop"', 'operator recovered'])
call s:Group(1, 'Loop.again', 'rastokJsonString')
call s:Group(1, 'true', 'rastokJsonConstant')
call s:Group(1, '2', 'rastokJsonNumber')
call s:Group(2, 'stop', 'rastokString')
call s:Group(3, 'recovered', 'rastokName')

call s:Buffer(['val value = "escaped \" quote // text"', 'operator recovered', 'val value = Text! {', 'Loop.again', '}'])
call s:Group(1, '// text', 'rastokString')
call s:Group(2, 'recovered', 'rastokName')
call s:Group(4, 'Loop.again', 'rastokManager')

call s:Buffer(['val raw = text! {', 'Loop.again // payload'])
call s:Group(2, 'Loop.again', 'rastokRawString')

" Editing a reader opener must invalidate the old lexical state.
call s:Buffer(['val raw = text! {', 'on in { Loop.done }', '}', 'operator recovered'])
call s:Group(2, 'Loop.done', 'rastokRawBrace')
call setline(1, 'val raw = {')
call s:Group(2, 'Loop.done', 'rastokManager')
call setline(1, 'val raw = text! {')
call s:Group(2, 'Loop.done', 'rastokRawBrace')

for example in sort(glob(s:root . '/examples/*.rkg', 0, 1))
  call s:Buffer(readfile(example) + ['operator tokenizer_sentinel'])
  call s:Group(line('$'), 'tokenizer_sentinel', 'rastokName')
endfor
execute b:undo_ftplugin
call assert_equal(&g:commentstring, &l:commentstring)
call assert_equal(&g:comments, &l:comments)
if !empty(v:errors)
  call writefile(v:errors, $RASTOK_EDITOR_TEST_LOG)
  cquit
endif
call writefile([printf('%d highlighting/detection checks passed', s:checks)], $RASTOK_EDITOR_TEST_LOG)
qa!
