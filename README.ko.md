# quota-cistern

[English](README.md)

> 코딩 에이전트를 위한 예산 기반 워크로드 스케줄러.  
> 위임 가능한 코딩 작업을 잔여 사용량이 있을 때 격리된 에이전트에게 맡겨 무인으로 실행합니다.

**상태:** 초기 개발 (`v0.1.0` 개발 중) · 아직 사용 가능한 릴리스는 없습니다.

---

## quota-cistern을 사용해야 하는 이유

> 세션 한도는 가득 채우는데 주간 사용량은 절반을 못채우는 비운의 개발자를 위하여...

구독형 AI 요금제로 에이전틱 코딩을 하면 사람이 방향·기준을 정하는 단계와 에이전트가 실행하는 단계가 같은 한도를 두고 경합합니다. 집중 작업 시간대에는 한도가 소진되고 그 외 시간대(수면·부재)의 잔여 한도는 낭비됩니다.

quota-cistern은 위임 가능한 실행을 잔여 사용량이 있을 때 처리해 한정된 한도를 낭비 없이 운용하도록 돕습니다. 사용량을 고정된 순서로 소비하는 작업 큐가 아니라 사용량 자체를 스케줄링 가능한 자원으로 다루는 것이 핵심입니다.

---

## 사용법

작업은 제목과 에이전트에게 줄 지시문으로 이루어집니다.

```console
$ cistern task add --title "refactor utils" --instruction "tidy up src/utils"
task:1 added to backlog
```

실행할 때는 할당량을 얼마나 쓸지와 얼마 동안 할지를 선언합니다. 명령은 바로 반환되고 세션은 계속 실행됩니다.

```console
$ cistern run --usage 50% --time 8h
session:1 running (2 tasks assigned to start)
  budget:  usage 50% · time 8h
```

작업의 결과와 무관하게 각자의 브랜치에 기록이 남습니다.

```console
$ cistern review ls
✓  task:1  refactor utils  session:1  → cistern/1  Completed    3 commits
⚠  task:3  update docs     session:1  → cistern/3  Interrupted  1 commit

$ cistern apply 1
$ cistern discard 3
```

`apply`는 작업의 변경을 작업 트리에 반영하고 `discard`는 목록에서 제외합니다. 어느 쪽도 커밋이나 push, 병합, 삭제를 하지 않습니다.

명령과 플래그, 종료 코드는 [CLI 명세](docs/cli.ko.md)에 전부 있습니다.

---

## 기여

초기 개발 단계이며 제안은 언제든 환영합니다. 개발 환경과 컨벤션, 코드 구성은 [CONTRIBUTING.ko.md](CONTRIBUTING.ko.md)에 있습니다.
