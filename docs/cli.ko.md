# quota-cistern 0.1.0 — CLI 명세

[English](cli.md)

## 1. 전역 규약

### 공통 플래그

모든 커맨드에 적용된다.


| 플래그            | 값 | 기본 | 설명                                |
| -------------- | - | -- | --------------------------------- |
| `-h`, `--help` | — | —  | 사용법을 stdout에 출력하고 종료 (코드 0) |


최상위 `cistern --version`은 버전을 출력한다. 서브커맨드 없이 `cistern`을 실행하거나 인자가 잘못되면 사용법을 stderr에 출력하고 코드 2로 끝낸다.

### 코어

모든 명령이 저장된 것을 관리하는 코어에 요청한다. 실행 중인 코어가 없으면 명령이 코어를 실행하고 하려던 일을 이어가며, 그 코어는 명령이 끝난 뒤에도 계속 실행된다. `--version`은 예외이며 양쪽 버전이 맞는지 보고하는 명령이므로 코어를 실행하지 않는다.

코어는 명령줄 옆에서 먼저 찾고 없으면 `PATH`에서 찾는다. 코어를 실행하지 못하거나 실행한 코어가 답하기 전에 종료하면 이유를 출력하고 코드 5로 끝낸다.

코어가 출력하는 내용은 `$XDG_STATE_HOME/cistern/daemon.log`에 기록하며 이 변수가 없으면 `~/.local/state/cistern/daemon.log`에 기록한다. 사람이 직접 실행한 코어는 터미널에 출력한다.

### 종료 코드


| 코드  | 의미       | 설명                |
| --- | -------- | ----------------- |
| 0   | 성공       | —                 |
| 1   | 일반 실패    | 연산이 거부됨           |
| 2   | 사용법 오류   | 인자·플래그 잘못         |
| 3   | 대상 없음    | 세션·작업 ID를 찾을 수 없음 |
| 4   | 상태 충돌    | 현재 상태에서 불가능한 연산   |
| 5   | 코어 내부 오류 | 코어를 실행하지 못했거나, 실행한 코어가 답하기 전에 종료했거나, 코어와 버전이 맞지 않거나, 요청 처리 중 실패함 |


### 출력

출력은 텍스트다. 각 커맨드 절의 출력 표는 그 커맨드가 답하는 필드를 정의하며, 그것이 서피스에 어떻게 도달하는지는 [IPC 문서](ipc.md)에 있다. 값이 없으면 `(none)`처럼 괄호로 표시하며, 라벨이 붙은 필드든 출력 전체든 같다.

### 식별자 표기

- 세션: `session:<n>` (예: `session:1`)
- 작업: `task:<n>` (예: `task:1`)
- 브랜치: `cistern/<taskid>` (코어가 작업당 생성)

`<n>`은 단조 증가하는 정수다. 작업·세션이 각각 독립 시퀀스이며 재사용 없이 생애에 걸쳐 유일하다. 커맨드 인자로는 접두어를 생략할 수 있다.

### 상태

작업(task) 상태:


| 상태            | 뜻                                   |
| ------------- | ----------------------------------- |
| `Pending`     | 백로그에 등록됨. 편성 전                      |
| `Running`     | 세션에 편성되어 실행 중                       |
| `Completed`   | 완수. 결과를 브랜치에 보존                     |
| `Interrupted` | 예산 하드락 또는 사용자 중단으로 종료. 진행분을 브랜치에 보존 |
| `Error`       | 실행 중 실패. 진행분이 있으면 브랜치에 보존           |


종료 상태(`Completed`·`Interrupted`·`Error`)는 모두 브랜치를 남겨 심사 대상이 된다. 처분 결과 `disposition`은 작업 상태와 별개로 기록한다.

작업의 `reason`: `budget hardlock` · `vendor limit` · `task ceiling` · `interrupted` · 실행 실패 사유.

`task ceiling`은 작업이 자기 상한만큼 소비해 멈춘 것으로, 세션은 계속된다. 이 상한은 사용자가 지정하지 않는다.

세션(session) 상태:


| 상태        | 뜻          |
| --------- | ---------- |
| `running` | 무인 루프 실행 중 |
| `stopped` | 종료됨        |


`stopped_reason`: `budget hardlock` · `vendor limit` · `observation unreadable` · `interrupted` · `all done`(편성분 전부 종료) · `error`.

`budget hardlock`은 선언한 예산이 소진된 것이고, `vendor limit`은 벤더가 한도로 실행을 막은 것이며, `observation unreadable`은 소비량을 읽을 수 없어 멈춘 것이다.

결과 브랜치는 도구가 지우거나 옮기지 않으며 push·병합·정리는 사용자가 수행한다.

### 목록 출력

목록 커맨드(`backlog` · `session ls` · `review ls`)는 항목이 없어도 성공(0)으로 끝나며 아무것도 출력하지 않는다.

## 2. 커맨드

커맨드는 루프 흐름을 따라 네 묶음(작업·백로그 / 세션·실행 / 관측 / 심사와 처분)으로 나뉘고, 설정은 이들과 별개로 실행 전에 둔다.

### 2.1 작업 · 백로그

작업은 어느 저장소에서 등록됐는지를 기록한다. 코어는 커맨드를 실행한 디렉터리에서 위로 올라가며 저장소 루트를 찾고, 저장소가 아니면 커맨드를 거부한다.

백로그는 `$XDG_DATA_HOME/cistern/backlog.json`에 저장한다. 그 변수가 없으면 `~/.local/share/cistern/backlog.json`이다.

#### `cistern task add`

작업을 백로그에 `Pending`으로 등록한다. 세션에 직접 배정하지 않으며, 어느 세션에 편성될지는 세션을 열 때 코어가 정한다.

```
cistern task add --title <T> --instruction <I> [--branch <B>] [--after <task>] [--model <M>]
```

**인자**


| 이름                  | 필수  | 형식    | 설명                              |
| ------------------- | --- | ----- | ------------------------------- |
| `--title <T>`       | 예   | 문자열   | 작업 제목                           |
| `--instruction <I>` | 예   | 문자열   | 에이전트 지시. `-`이면 stdin에서 읽음       |
| `--branch <B>`      | 아니오 | 브랜치명  | 작업이 시작할 브랜치. 기본 `main`이고, `--after`를 주면 선행 작업의 결과 브랜치 |
| `--after <task>`    | 아니오 | ID    | 선행 작업. 그 작업이 끝나기 전에는 편성되지 않는다 |
| `--model <M>`       | 아니오 | 모델 이름 | 이 작업에 쓸 모델. 생략하면 세션의 `--model`  |


둘을 함께 줄 수 있다. 그때 작업은 선행 작업을 기다리되 지정한 브랜치에서 시작한다.

`--after`를 주면 선행 작업이 `Completed`가 될 때까지 편성 대상이 아니며, 다른 종료 상태로 끝나면 `Pending`으로 남는다.

**출력**


| 필드            | 타입     | 설명               |
| ------------- | ------ | ---------------- |
| `id`          | string | 작업 식별자           |
| `title`       | string | 작업 제목            |
| `base_branch` | string | 기준 브랜치           |
| `after`       | string | 선행 작업. 없으면 null  |
| `model`       | string | 지정한 모델. 없으면 null |
| `repository`  | string | 작업을 등록한 저장소       |
| `state`       | enum   | 생성 직후 `Pending`  |


**종료 코드**


| 코드  | 조건                          |
| --- | --------------------------- |
| 0   | 성공                          |
| 2   | 인자 오류 (예: `--title` 누락)     |
| 3   | `--after`가 가리키는 작업 없음       |
| 4   | 저장소 안에서 실행하지 않음             |
| 5   | 코어 오류                       |


**예시**

```console
$ cistern task add --title "리팩터 X" --instruction "src/utils 정리"
task:1 added to backlog
  title:  리팩터 X
  branch: main (base)
  repo:   ~/work/api
```

#### `cistern task rm`

백로그에서 작업을 지운다. 아직 편성되지 않은 `Pending` 작업만 대상이며, 끝난 작업은 `discard`가 심사 큐에서 제외한다.

```
cistern task rm <task>
```

**출력**


| 필드      | 타입     | 설명    |
| ------- | ------ | ----- |
| `id`    | string | 지운 작업 |
| `title` | string | 작업 제목 |


**종료 코드**


| 코드  | 조건            |
| --- | ------------- |
| 0   | 성공            |
| 3   | 작업 없음         |
| 4   | `Pending`이 아님 |
| 5   | 코어 오류         |


**예시**

```console
$ cistern task rm 3
task:3 removed from backlog
```

#### `cistern backlog`

편성 전 Pending 작업(백로그)을 나열한다.

```
cistern backlog
```

**출력** — 항목 배열


| 필드            | 타입     | 설명     |
| ------------- | ------ | ------ |
| `id`          | string | 작업 식별자 |
| `title`       | string | 작업 제목  |
| `base_branch` | string | 기준 브랜치 |


한 줄에 한 항목으로 출력한다.

**종료 코드**


| 코드  | 조건    |
| --- | ----- |
| 0   | 성공    |
| 5   | 코어 오류 |


**예시**

```console
$ cistern backlog
○ task:1  리팩터 X          base main
○ task:2  통합테스트 추가     base main
○ task:3  README 갱신       base main
```

#### `cistern task show`

작업 하나의 상세를 출력한다.

```
cistern task show <task>
```

**출력**


| 필드            | 타입     | 설명                                                                             |
| ------------- | ------ | ------------------------------------------------------------------------------ |
| `id`          | string | 작업 식별자                                                                         |
| `session`     | string | 편성된 세션. 미편성이면 null                                                             |
| `state`       | enum   | 작업 상태                                                                          |
| `title`       | string | 작업 제목                                                                          |
| `base_branch` | string | 기준 브랜치                                                                         |
| `after`       | string | 선행 작업. 없으면 null                                                                |
| `model`       | string | 실행에 쓴 모델                                                                       |
| `repository`  | string | 작업을 등록한 저장소. 홈 디렉터리는 `~`로 표시                                                   |
| `branch`      | string | 결과 브랜치. 없으면 null                                                               |
| `reason`      | string | 종료 사유. 없으면 null                                                                |
| `worktree`    | string | 작업 공간 경로. 정리된 뒤에는 null                                                         |
| `disposition` | enum   | `applied` · `discarded` · 아직 처분하지 않았으면 null                                    |


**종료 코드**


| 코드  | 조건    |
| --- | ----- |
| 0   | 성공    |
| 3   | 작업 없음 |
| 5   | 코어 오류 |


**예시**

```console
$ cistern task show 2
task:2  Interrupted
  session:     session:1
  title:       테스트 추가
  base:        main
  after:       (none)
  repo:        ~/work/api
  branch:      cistern/2
  reason:      budget hardlock
  worktree:    ~/.local/share/cistern/worktrees/2
  disposition: (none)
```

### 2.2 세션 · 실행

세션은 `$XDG_DATA_HOME/cistern/sessions.json`에 저장하거나, `XDG_DATA_HOME`이 없을 경우 `~/.local/share/cistern/sessions.json`에 저장한다.

작업은 `$XDG_DATA_HOME/cistern/worktrees` 아래에 `git worktree`로 만드는 별도 체크아웃에서 실행한다.

#### `cistern run`

예산을 선언하고 세션의 무인 루프를 기동한다. 논블로킹이라 즉시 반환한다.

```
cistern run --usage <N> --time <T> [--model <M>]
```

**인자**


| 이름            | 필수  | 형식          | 예시                | 설명                                                   |
| ------------- | --- | ----------- | ----------------- | ---------------------------------------------------- |
| `--usage <N>` | 예   | 백분율 또는 토큰 수 | `50%` · `2M`      | `%`면 벤더의 5시간 한도에 대한 몫, `%` 없으면 토큰 수                  |
| `--time <T>`  | 예   | 기간          | `8h` · `2h30m`    | 시간 한도                                                |
| `--model <M>` | 아니오 | 모델 이름       | `opus` · `sonnet` | 모델을 지정하지 않은 작업의 기본값. 생략 시 vendor 기본                  |


기동하면 코어가 백로그에서 일부를 편성해 병렬로 실행한다. 편성은 동적이어서 작업이 끝날
때마다 그 작업이 실제로 소비한 양을 근거로 하나를 더 편성할지 판단하며, 편성되지 않은 작업은
백로그에 `Pending`으로 남는다. 선행 작업이 `Completed`가 되지 않은 작업은 편성 대상이 아니다.

세션 내 작업은 병렬로 실행되지만 세션 자체는 동시에 하나만 실행된다. 다른 세션이 실행 중이면
거부한다.

`%`는 벤더의 5시간 한도에 대한 몫으로 1에서 100 사이의 정수이고, 토큰 수는
`K`(=1,000)·`M`(=1,000,000) 접미사를 받는 정수다.

보고되는 소비량은 근사값이다.

사용량과 시간 가운데 먼저 소진되는 쪽에서 세션이 자동으로 정지한다. 소비 표시는 선언 단위를 따라 `%`로 선언하면 `%`로, 토큰으로 선언하면 토큰으로 출력한다.

**출력**


| 필드         | 타입     | 설명                               |
| ---------- | ------ | -------------------------------- |
| `session`  | string | 생성·기동한 세션                        |
| `state`    | enum   | `running`                        |
| `assigned` | int    | 기동 시점에 편성된 작업 수. 편성이 동적이므로 이후 증가 |
| `budget`   | object | 선언한 예산 (usage·time)              |


**종료 코드**


| 코드  | 조건                              |
| --- | ------------------------------- |
| 0   | 기동 성공                           |
| 1   | 편성할 작업이 하나도 없음                  |
| 2   | 인자 형식 오류 (예: `--time 8x`)       |
| 4   | 다른 세션이 실행 중                              |
| 5   | 코어 오류                           |


**예시**

```console
$ cistern run --usage 50% --time 8h
session:1 running (2 tasks assigned to start)
  budget:  usage 50% · time 8h
  observe: cistern trace <task> --follow
  stop:    cistern interrupt
```

#### `cistern interrupt`

실행 중인 세션을 중단한다. 세션은 동시에 하나만 실행되므로 대상을 지정하지 않으며, 실행 중이던 작업은 `Interrupted`로 종료한다.

```
cistern interrupt
```

**출력**


| 필드                  | 타입     | 설명                     |
| ------------------- | ------ | ---------------------- |
| `session`           | string | 중단한 세션                 |
| `state`             | enum   | `stopped`              |
| `interrupted_tasks` | array  | Interrupted로 종료된 작업 id |
| `consumed`          | object | 실측 소비 (usage·time)      |


**종료 코드**


| 코드  | 조건          |
| --- | ----------- |
| 0   | 성공          |
| 4   | 실행 중인 세션 없음 |
| 5   | 코어 오류       |


**예시**

```console
$ cistern interrupt
session:1 interrupted
  task:2 → Interrupted
  consumed 38% · time 2h05m
```

#### `cistern session ls`

세션을 최신순으로 나열한다.

```
cistern session ls [--page <N>] [--limit <M>]
```

**인자**


| 이름            | 필수  | 형식    | 설명             |
| ------------- | --- | ----- | -------------- |
| `--page <N>`  | 아니오 | 정수 ≥1 | 페이지 번호. 기본 1   |
| `--limit <M>` | 아니오 | 정수    | 페이지당 개수. 기본 20 |


**출력** — 항목 배열


| 필드           | 타입     | 설명      |
| ------------ | ------ | ------- |
| `id`         | string | 세션 식별자  |
| `state`      | enum   | 세션 상태   |
| `consumed`   | string | 소비량     |
| `task_count` | int    | 세션 작업 수 |
| `updated_at` | string | 갱신 시각   |


최신 세션부터 한 줄에 하나씩 출력한다.

**종료 코드**


| 코드  | 조건                    |
| --- | --------------------- |
| 0   | 성공                    |
| 2   | 인자 오류 (예: `--page 0`) |
| 5   | 코어 오류                 |


**예시**

```console
$ cistern session ls
session:3  running    usage 12%   2 tasks   just now
session:1  stopped    usage 50%   3 tasks   3h ago
```

#### `cistern session show`

세션 하나의 상세를 출력하며 내부 작업 목록을 포함한다.

```
cistern session show <session>
```

**출력**


| 필드               | 타입     | 설명                                        |
| ---------------- | ------ | ----------------------------------------- |
| `budget`         | object | 선언한 예산 (usage·time)                       |
| `consumed`       | object | 실측 소비 (usage·time)                        |
| `stopped_reason` | enum   | 정지 사유. 실행 중이면 null                        |
| `resets_at`      | string | 벤더 한도가 재설정되는 시각. `vendor limit`으로 정지했을 때만 |
| `updated_at`     | string | 갱신 시각                                     |
| `tasks`          | array  | 세션 작업. 각 항목은 id·state·title·branch·reason |


첫 줄 괄호에 `stopped_reason`을 표시하며, 실행 중이면 `running`으로 표시하고 사유를 표시하지 않는다.

**종료 코드**


| 코드  | 조건    |
| --- | ----- |
| 0   | 성공    |
| 3   | 세션 없음 |
| 5   | 코어 오류 |


**예시**

```console
$ cistern session show 1
session:1  stopped (budget hardlock)
  budget:   usage 50% · time 8h
  consumed: usage 50% · time 3h12m
  tasks:
    ✓  task:1  Completed    리팩터 X       → cistern/1
    ⚠  task:2  Interrupted  테스트 추가     → cistern/2  (budget hardlock)
    ✕  task:4  Error        문서 갱신       → cistern/4  (process died)
```

### 2.3 관측

#### `cistern trace`

작업의 트레이스를 조회한다. 실행 중이면 진행 중인 것을, 종료 후면 보관된 것을 반환한다.

```
cistern trace <task> [--follow] [--since <cursor>]
```

**인자**


| 이름                 | 필수  | 형식  | 설명                                  |
| ------------------ | --- | --- | ----------------------------------- |
| `<task>`           | 예   | ID  | 대상 작업                               |
| `--follow`         | 아니오 | 플래그 | 실행 중이면 새 트레이스를 이어서 표시. 종료되면 자동으로 끝남 |
| `--since <cursor>` | 아니오 | 커서  | 해당 지점 이후만 출력                        |


트레이스는 에이전트 산출물이며 코어는 이를 append-only로 보관한다. 이 커맨드는 현재까지의 이벤트와 다음 커서를 반환한다.

**출력**


| 필드       | 타입     | 설명                          |
| -------- | ------ | --------------------------- |
| `events` | array  | 시간순 트레이스 이벤트                |
| `cursor` | string | 다음 조회 시작점                   |
| `done`   | bool   | 종료 상태에 도달해 더 이상 늘지 않으면 true |


이벤트를 한 줄씩 출력하며, 일어난 시각과 내용을 함께 적는다.

**종료 코드**


| 코드  | 조건    |
| --- | ----- |
| 0   | 성공    |
| 3   | 작업 없음 |
| 5   | 코어 오류 |


**예시**

```console
$ cistern trace 1
[11:19:36] I'll start implementing app/scoring.py based on the specification.
[11:19:37] Read SPEC.md
[11:20:13] Read app/scoring.py
[11:20:13] failed: File does not exist
[11:20:20] Write app/scoring.py
[11:20:23] Bash python3 -m pytest tests/test_scoring.py -v
[11:20:36] All done! I've implemented app/scoring.py with two functions.
```

#### `cistern diff`

작업이 만든 브랜치의 변경 내용을 출력한다.

```
cistern diff <task> [--stat]
```

**인자**


| 이름       | 필수  | 형식  | 설명                |
| -------- | --- | --- | ----------------- |
| `<task>` | 예   | ID  | 대상 작업             |
| `--stat` | 아니오 | 플래그 | 파일별 요약(변경 파일·증감)만 |


**출력**


| 필드       | 타입     | 설명                           |
| -------- | ------ | ---------------------------- |
| `base`   | string | 기준 브랜치                       |
| `branch` | string | 결과 브랜치                       |
| `files`  | array  | 파일별 `path`·`added`·`removed` |
| `patch`  | string | unified diff                 |


표준 unified diff를 출력하고 `--stat`이면 파일 요약만 출력한다. 변경이 없으면 `(no changes)`를 출력한다.

**종료 코드**


| 코드  | 조건                  |
| --- | ------------------- |
| 0   | 성공                  |
| 1   | 브랜치에 변경이 없거나 브랜치를 읽을 수 없음 |
| 3   | 작업 없음               |
| 5   | 코어 오류               |


**예시**

```console
$ cistern diff 1 --stat
 src/utils/index.ts   | 12 +++---
 src/utils/graph.ts   | 40 ++++++++++----
 2 files changed, 34 insertions(+), 18 deletions(-)
```

### 2.4 심사와 처분

`review ls`가 처분을 기다리는 작업을 나열하고 `apply`와 `discard`가 처분하며, 어느 쪽도 브랜치를
바꾸지 않는다.

#### `cistern review ls`

처분을 기다리는 작업을 세션 구분 없이 모아 나열한다. `Completed`·`Interrupted`·`Error`가 함께 나열된다.

```
cistern review ls
```

**출력** — 항목 배열


| 필드             | 타입     | 설명                        |
| -------------- | ------ | ------------------------- |
| `id`           | string | 작업 식별자                    |
| `title`        | string | 작업 제목                     |
| `session`      | string | 출처 세션                     |
| `branch`       | string | 결과 브랜치                    |
| `state`        | enum   | 종료 상태                     |
| `commit_count` | int    | 결과 브랜치의 커밋 수              |
| `base_ahead`   | int    | 갈라져 나온 뒤 기준 브랜치가 앞서간 커밋 수 |


한 줄에 한 작업으로 출력한다.

처분한 작업은 큐에서 제외한다. `base_ahead`는 조회할 때마다 계산한다. 브랜치를 읽을 수 없는 작업도
큐에 남으며, 이때 두 개수는 비어 있다.

**종료 코드**


| 코드  | 조건    |
| --- | ----- |
| 0   | 성공    |
| 5   | 코어 오류 |


**예시**

```console
$ cistern review ls
✓  task:5  웹훅 서명 검증 추가   session:1  → cistern/5  Completed    3 commits
⚠  task:6  리포트 생성 테스트   session:2  → cistern/6  Interrupted  1 commit · base +2
```

#### `cistern apply`

결과 브랜치의 변경을 작업을 등록한 저장소의 작업 트리에 적용한다. 커밋하지 않으며 브랜치를 옮기거나 지우지
않는다.

```
cistern apply <task>
```

적용 범위는 기준 브랜치와 결과 브랜치가 갈라진 지점부터 결과 브랜치까지다. `diff`와 같은
기준이다.

작업 트리에 커밋되지 않은 변경이 있으면 거부하고, 적용 중 충돌이 발생하면 아무것도 적용하지
않는다. 읽는 대상이 브랜치이므로 워크트리가 정리된 뒤에도 동작한다.

**출력**


| 필드       | 타입     | 설명                               |
| -------- | ------ | -------------------------------- |
| `task`   | string | 처분한 작업                           |
| `branch` | string | 읽은 결과 브랜치                        |
| `files`  | array  | 적용한 파일별 `path`·`added`·`removed` |


**종료 코드**


| 코드  | 조건                                     |
| --- | -------------------------------------- |
| 0   | 성공                                     |
| 1   | 적용할 변경이 없거나, 이미 작업 트리에 있거나, 충돌로 거부됨      |
| 3   | 작업 없음                                  |
| 4   | 아직 종료되지 않은 작업이거나 작업 트리에 커밋되지 않은 변경이 있음 |
| 5   | 코어 오류                                  |


**예시**

```console
$ cistern apply 5
task:5 applied to working tree
  src/webhook/verify.ts   +64 -3
  src/webhook/index.ts     +8 -1
  (nothing committed · review and commit in your own environment)
```

#### `cistern discard`

작업을 심사 큐에서 제외한다. 브랜치와 워크트리, 작업 상태 어느 것도 바꾸지 않는다.

```
cistern discard <task>
```

결과 브랜치는 그대로 남으므로 처분한 뒤에도 `task show`로 조회하고 `apply`할 수 있다.

**출력**


| 필드       | 타입     | 설명            |
| -------- | ------ | ------------- |
| `task`   | string | 처분한 작업        |
| `branch` | string | 그대로 남는 결과 브랜치 |


**종료 코드**


| 코드  | 조건            |
| --- | ------------- |
| 0   | 성공            |
| 3   | 작업 없음         |
| 4   | 아직 종료되지 않은 작업 |
| 5   | 코어 오류         |


**예시**

```console
$ cistern discard 6
task:6 discarded
  branch cistern/6 is kept
```

### 2.5 설정

#### `cistern config`

vendor를 설정한다.

```
cistern config set <key> <value>
cistern config get [<key>]
```

**키**


| 키             | 값                                       | 설명                                  |
| ------------- | --------------------------------------- | ----------------------------------- |
| `vendor`      | `claude`                                | 실행 에이전트 vendor. 0.1.0은 `claude`만 지원 |


설정은 `$XDG_CONFIG_HOME/cistern/config.toml`에 저장한다. 그 변수가 없으면 `~/.config/cistern/config.toml`이다.

**출력**

`set`은 적용된 키와 값을, `get`은 현재 설정을 출력하며 키를 주면 그 값만 출력한다.

**종료 코드**


| 코드  | 조건         |
| --- | ---------- |
| 0   | 성공         |
| 2   | 알 수 없는 키·값 |
| 5   | 코어 오류      |


**예시**

```console
$ cistern config set vendor claude
vendor = claude

$ cistern config get
vendor: claude
```

