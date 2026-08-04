# 기여 가이드

[English](CONTRIBUTING.md)

quota-cistern은 초기 개발 단계입니다. 제안·논의는 언제든 환영입니다. 

## 논의와 제안

버그·아이디어·질문이 있다면 [이슈](../../issues)를 작성해 주세요.\
논의가 필요하다면 [Discussions](../../discussions)에 작성해 주세요.\
모든 코드 작업은 논의 후에 진행됩니다. 기여하실 의향이 있으시다면 작업을 할당받은 후 작업해주세요.

## 풀 리퀘스트

1. 저장소를 포크하고 `main`에서 브랜치를 만듭니다.
2. 변경은 하나의 논리 단위로 작게 유지합니다.
3. PR을 열고 관련 이슈를 링크합니다.

### 브랜치 네이밍

`<type>/<간단한-설명>` 형식을 씁니다. `<type>`은 아래 커밋 규칙과 동일하게 맞추고 
설명은 소문자 kebab-case로 적습니다. 관련 이슈가 있으면 번호를 앞에 붙일 수 있습니다.

```
feat/budget-hardlock
fix/session-race
docs/cli-exit-codes
feat/12-budget-hardlock   # 이슈 #12
```

> `cistern/*` 접두어는 도구가 작업별로 생성하는 결과 브랜치용으로 예약되어 있으니
> 기여 브랜치에는 쓰지 않습니다.

### 커밋 · PR 제목

커밋 메시지와 PR 제목은 [Conventional Commits](https://www.conventionalcommits.org/)를 따릅니다.
`<type>`은 `feat` · `fix` · `docs` · `refactor` · `test` · `chore`를 씁니다 (예: `feat: …`, `fix: …`).

## 개발 환경

빌드·테스트 절차는 기술 스택이 확정되면 이 문서에 추가합니다.

## 행동 강령

참여할 경우 [행동 강령](./CODE_OF_CONDUCT.ko.md)에 동의하는 것으로 간주합니다. 
