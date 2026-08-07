# quota-cistern

[English](README.md)

> 선언한 예산 안에서 코딩 에이전트에게 작업을 맡기고 결과를 브랜치로 받아 확인한 뒤 반영하는 도구입니다. 개발 초기라 아직 사용할 수 있는 릴리스가 없습니다.

집중해서 작업한 날일수록 세션 한도를 빨리 소진합니다. 무엇을 만들지 정하고 구조를 잡는 데 시간을 쓰고 나면 정작 그것을 코드로 옮길 차례에는 한도가 얼마 남지 않습니다. 밤에는 한도가 리셋되지만 그 시간을 작업에 쓰기는 어렵고 쓰지 못한 몫은 이월되지 않고 사라집니다.

한도를 쓰려면 사람이 함께 있어야 하기 때문입니다. 에이전트를 부르고 결과를 확인하고 다음을 지시하는 일에 사람의 시간이 들어가므로 한도는 사람이 작업하는 시간 안에서만 소비됩니다. 방향이 정해진 뒤의 구현은 사람이 지켜보지 않아도 되는 일인데 이 제약은 그런 작업에도 똑같이 적용됩니다.

quota-cistern은 그 제약을 없앱니다. 위임할 작업을 등록하고 자리를 비우기 전에 얼마까지 쓸지 선언하면 남은 한도 안에서 작업을 처리하고 선언한 값에 이르면 중단합니다. 돌아오면 작업마다 결과가 브랜치에 보존되어 있습니다.

## 워크플로우

### 작업 등록

작업은 제목과 지시문으로 등록하며 백로그에 있는 작업은 세션이 열릴 때 배정됩니다. `--after`로 선행 작업을 지정하면 그 작업의 결과 브랜치를 기준으로 이어서 진행하고 기준 브랜치와 모델은 작업마다 따로 지정할 수 있습니다. 예산을 비율로 선언하려면 `config set plan`으로 요금제를 지정해야 합니다.

### 무인 실행

사용량과 시간을 선언하면 먼저 소진되는 쪽에서 세션이 끝나는데, 그 시점에 새 작업을 배정하지 않는 데 그치지 않고 실행 중인 작업까지 중단합니다. 백로그 전체를 한 번에 실행하지는 않고 작업 하나가 끝날 때마다 실제 소비량을 확인해 하나를 더 배정할지 정하는데, 이는 작업마다 소비량이 다르고 실행 전에는 알 수 없기 때문입니다.

작업은 각자 별도의 작업 공간과 브랜치에서 병렬로 실행되므로 서로의 결과에 영향을 주지 않으며 `interrupt`로 세션을 중단하더라도 그때까지 진행된 부분은 브랜치에 보존됩니다.

### 결과 검토

완료든 중단이든 실패든 끝난 작업은 모두 리뷰 대상이 되고 세션이 왜 중단됐는지도 함께 기록됩니다. 에이전트의 출력은 작업마다 보관되어 실행 중에도 끝난 뒤에도 읽을 수 있으며 변경 내용은 `diff`로 확인하거나 `--stat`으로 파일별 요약만 볼 수 있습니다.

`apply`는 변경을 작업 트리에 적용할 뿐 커밋하지 않고 `discard`로 제외하더라도 브랜치는 삭제되지 않아 나중에 다시 반영할 수 있습니다. 결과 브랜치는 만든 뒤 도구가 수정하지 않으며 병합이나 push도 하지 않으므로 그 브랜치를 어떻게 할지는 사용자가 정합니다.

### 사용 예시

```console
$ cistern task add --title "refactor utils" --instruction "tidy up src/utils"
task:1 added to backlog
  title:  refactor utils
  branch: main (base)

$ cistern task add --title "update docs" --instruction "document the new API"
task:2 added to backlog
  title:  update docs
  branch: main (base)

$ cistern run --usage 50% --time 8h
session:1 running (2 tasks assigned to start)
  budget:  usage 50% · time 8h
  observe: cistern trace <task> --follow
  stop:    cistern interrupt

# 세션이 끝난 뒤

$ cistern review ls
✓  task:1  refactor utils  session:1  → cistern/1  Completed    3 commits
⚠  task:2  update docs     session:1  → cistern/2  Interrupted  1 commit

$ cistern apply 1
task:1 applied to working tree
  (nothing committed · review and commit in your own environment)
```

## 명령어

| 명령어 | 하는 일 |
| --- | --- |
| `config set` · `config get` | 벤더와 요금제 |
| `task add` · `task rm` · `task show` · `backlog` | 작업 등록과 조회 |
| `run` · `interrupt` | 예산 선언과 실행, 중단 |
| `session ls` · `session show` | 세션 조회 |
| `trace` · `diff` | 진행 상황과 변경 내용 |
| `review ls` · `apply` · `discard` | 검토와 처분 |

모두 `-o json`으로 기계가 읽을 형식을 출력합니다. 인자와 출력, 종료 코드는 [CLI 명세](docs/cli.md)에 있습니다.

## 시작하기

아직 릴리스가 없습니다. 소스에서 빌드하는 방법은 [CONTRIBUTING](CONTRIBUTING.md)에 있습니다.

0.1.0이 다루는 범위는 벤더 `claude` 하나와 장비 한 대이며, 세션은 한 번에 하나만 실행되고 `run`을 실행한 시점에 열립니다.

## 기여

초기 개발 단계이며 제안은 언제든 환영합니다. 개발 환경과 컨벤션은 [CONTRIBUTING](CONTRIBUTING.md)에, 구조와 설계 결정은 [docs/](docs/)에 정리해 두었고, 지금 무엇을 만들고 있는지는 [v0.1.0 마일스톤](https://github.com/BuildWithYJ/quota-cistern/milestone/1)에서 볼 수 있습니다.

## 라이선스

MIT입니다. [LICENSE](LICENSE)를 보세요.
