" Rastok source highlighting. The compiler remains the syntax/type authority.
if exists('b:current_syntax')
  finish
endif
syntax case match

syntax keyword rastokDeclaration graph operator connector lab simulation experiment theorem scenario mock type input output val var nextgroup=rastokName skipwhite
syntax match rastokName /[A-Za-z_][A-Za-z0-9_]*/ contained
syntax keyword rastokKeyword on start where within last match nothing stop expect advance crash after restart origin is as in
syntax keyword rastokConstant true false null
syntax match rastokVariable /\<\%(data\|meta\|instance\|lifetime\|closing\|behavior\|idempotent\|json_profile\)\>/
syntax match rastokExtension /\<\%(App\|Loop\|Retry\|Saga\)\>/
syntax keyword rastokConnector Process File HttpClient HttpServer WebSocketClient WebSocketServer
syntax match rastokType /\<\%(struct\|union\|enum\|map\|array\|table\|bool\|int\|long\|uint\|float\|double\|number\|decimal\|text\|bytes\|variant\|date\|time\|uuid\|timestamp\|timestamp_ntz\|duration\|media_type\)\>/
syntax match rastokFunction /\<\%(env\|secret\|asset\|duration\|timestamp\|decimal\|media_type\|size\|has\|now\)\>\ze\s*(/
syntax match rastokType /\<decimal\>\ze\s*(\s*[0-9]\+\s*,/
syntax match rastokNumber /\<[0-9]\+\%([u]\|\.[0-9]\+\)\?\>/
syntax match rastokOperator /<->\|->\|=>\|==\|!=\|<=\|>=\|&&\|||\|[=+*\/%!?<>|.-]/
syntax match rastokAnnotation /@[A-Za-z_][A-Za-z0-9_]*/
syntax match rastokType /@struct\>/
syntax match rastokManager /\<Loop\.\%(next\|again\|done\|state\|fuel\|delay\|measure\|entry\|terminal\|expired\)\>/
syntax match rastokManager /\<Retry\.\%(attempts\|delay\|on\)\>/
syntax match rastokManager /\<\%([A-Za-z_][A-Za-z0-9_]*\.\)*instance\.stop\>/

" Strings end at a physical line, even when its last byte is a backslash.
syntax region rastokString start=/"/ skip=/\\[^\r\n]/ end=/"/ end=/$/ oneline
syntax region rastokString start=/'/ skip=/\\[^\r\n]/ end=/'/ end=/$/ oneline
syntax match rastokComment +//.*$+
syntax region rastokComment start=+/\*+ end=+\*/+

" Raw readers count braces; quotes and comment delimiters in them are data.
syntax region rastokRawString matchgroup=rastokReader start=/\<\%(text\|base64\)!\%([ \t]*(\s*"\%(\\.\|[^"\\]\)*"\s*)\)\?\_s*{/ end=/}/ contains=rastokRawBrace
syntax region rastokRawBrace start=/{/ end=/}/ contained contains=rastokRawBrace

syntax match rastokReader /\<json!/
syntax region rastokJsonObject matchgroup=rastokReader start=/\<json!\_s*{/ end=/}/ contains=@rastokJson
syntax region rastokJsonArray matchgroup=rastokReader start=/\<json!\_s*\[/ end=/\]/ contains=@rastokJson
syntax cluster rastokJson contains=rastokJsonString,rastokJsonNumber,rastokJsonConstant,rastokJsonNestedObject,rastokJsonNestedArray
syntax region rastokJsonNestedObject start=/{/ end=/}/ contained contains=@rastokJson
syntax region rastokJsonNestedArray start=/\[/ end=/\]/ contained contains=@rastokJson
syntax region rastokJsonString start=/"/ skip=/\\[^\r\n]/ end=/"/ end=/$/ oneline contained
syntax match rastokJsonNumber /-\?\<\%(0\|[1-9][0-9]*\)\%(\.[0-9]\+\)\?\%([eE][+-]\?[0-9]\+\)\?/ contained
syntax keyword rastokJsonConstant true false null contained

" Recompute lexical context from the start, including multiline raw readers.
syntax sync fromstart
highlight default link rastokDeclaration Keyword
highlight default link rastokName Identifier
highlight default link rastokKeyword Keyword
highlight default link rastokConstant Constant
highlight default link rastokVariable Identifier
highlight default link rastokExtension Type
highlight default link rastokConnector Type
highlight default link rastokType Type
highlight default link rastokFunction Function
highlight default link rastokNumber Number
highlight default link rastokOperator Operator
highlight default link rastokAnnotation PreProc
highlight default link rastokManager Function
highlight default link rastokString String
highlight default link rastokComment Comment
highlight default link rastokReader Function
highlight default link rastokRawString String
highlight default link rastokRawBrace String
highlight default link rastokJsonString String
highlight default link rastokJsonNumber Number
highlight default link rastokJsonConstant Constant
let b:current_syntax = 'rastok'
