//! Example pages for a demo instance: the knowledge base of the demo shop project and a
//! small team space, written over the same weeks as the demo issues.
//!
//! Every title and body comes in both languages of the demo, in full; a `[[link]]` in a
//! body names the page's title in the same language.

use chrono::{Duration, SecondsFormat, Utc};
use hub_app::db::{query_all, query_one};
use hub_app::{Actor, ApiError, ApiResult, App};
use hub_core::demo::{DemoLanguage, DemoText as Text};
use hub_core::Id;

use crate::pages::{self, PageEdit};
use crate::spaces::{self, NewSpace, SpaceRecord};

struct Page {
    title: Text,
    parent: Option<Text>,
    /// Index into the demo people; 0 is whoever set the instance up.
    author: usize,
    day: i64,
    body: Text,
}

const WEB_PAGES: &[Page] = &[
    Page {
        title: Text::new("Store architecture", "Architektura sklepu"),
        parent: None,
        author: 4,
        day: -27,
        body: Text::new(
            "The store has three parts: the browser application, the orders API and supporting services (payments, e-mails, search).\n\n## Components\n\n| Component | Technology | Responsible for |\n|---|---|---|\n| Frontend | React, Vite | catalogue, cart, customer account |\n| API | Rust, PostgreSQL | orders, prices, stock levels |\n| Payments | external gateway | cards and Pay by bank, see [[Payments]] |\n| Sessions | Redis (migration in progress) | sign-in, guest cart |\n\n## Order flow\n\n1. The customer adds products to the cart; a guest's cart lives in the session.\n2. On the order summary the API recalculates prices and discounts on its side.\n3. Once a payment method is chosen, the order gets the status *awaiting payment*.\n4. The gateway confirms the payment with a webhook; only then do we send the confirmation e-mail.\n\n## Sessions\n\nToday sessions are kept in the database. Moving them to Redis is described in [[ADR-001: Sessions in Redis]], and the work is tracked in WEB-13.\n\n## See also\n\n- [[Decisions]] — why things are the way they are\n- [[Production deployment]] — how all of this reaches production\n",
            "Sklep składa się z trzech części: aplikacji przeglądarkowej, API zamówień i usług pomocniczych (płatności, e-maile, wyszukiwarka).\n\n## Komponenty\n\n| Komponent | Technologia | Odpowiada za |\n|---|---|---|\n| Frontend | React, Vite | katalog, koszyk, konto klienta |\n| API | Rust, PostgreSQL | zamówienia, ceny, stany magazynowe |\n| Płatności | bramka zewnętrzna | karty i BLIK, zob. [[Płatności]] |\n| Sesje | Redis (w trakcie migracji) | logowanie, koszyk gościa |\n\n## Przepływ zamówienia\n\n1. Klient dodaje produkty do koszyka; koszyk gościa żyje w sesji.\n2. W podsumowaniu API przelicza ceny i rabaty po swojej stronie.\n3. Po wyborze metody płatności zamówienie dostaje status *oczekuje na płatność*.\n4. Bramka potwierdza płatność webhookiem; dopiero wtedy wysyłamy e-mail z potwierdzeniem.\n\n## Sesje\n\nSesje trzymamy dziś w bazie. Przeniesienie ich do Redis opisuje [[ADR-001: Sesje w Redis]], a pracę śledzi WEB-13.\n\n## Zobacz też\n\n- [[Decyzje]] — dlaczego jest tak, a nie inaczej\n- [[Wdrożenie produkcyjne]] — jak to wszystko trafia na produkcję\n",
        ),
    },
    Page {
        title: Text::new("Payments", "Płatności"),
        parent: None,
        author: 1,
        day: -13,
        body: Text::new(
            "All online payments go through one gateway (the reasoning: [[ADR-002: One gateway for cards and Pay by bank]]).\n\n| Method | Issue | State |\n|---|---|---|\n| Card (3-D Secure) | WEB-9 | in review |\n| Pay by bank | WEB-10 | in progress |\n| Cash on delivery | — | live since version 1.0 |\n\n## Rules\n\n- An order changes status **only** after the gateway's webhook, never when the customer returns from the payment page.\n- Every notification from the gateway is stored in full before we process it.\n- Amounts are counted in pence, never as floating-point numbers.\n\n## Subpages\n\n- [[Pay by bank — notification channel]]\n- [[Card gateway: sandbox]]\n",
            "Wszystkie płatności online idą przez jedną bramkę (uzasadnienie: [[ADR-002: Jedna bramka dla kart i BLIK]]).\n\n| Metoda | Zgłoszenie | Stan |\n|---|---|---|\n| Karta (3-D Secure) | WEB-9 | w przeglądzie |\n| BLIK | WEB-10 | w toku |\n| Za pobraniem | — | działa od wersji 1.0 |\n\n## Zasady\n\n- Zamówienie zmienia status **wyłącznie** po webhooku z bramki, nigdy po powrocie klienta ze strony płatności.\n- Każde powiadomienie z bramki zapisujemy w całości, zanim je przetworzymy.\n- Kwoty liczymy w groszach, bez liczb zmiennoprzecinkowych.\n\n## Podstrony\n\n- [[BLIK — kanał powiadomień]]\n- [[Bramka kartowa: sandbox]]\n",
        ),
    },
    Page {
        title: Text::new("Pay by bank — notification channel", "BLIK — kanał powiadomień"),
        parent: Some(Text::new("Payments", "Płatności")),
        author: 2,
        day: -6,
        body: Text::new(
            "Pay by bank confirms a payment over a different channel than cards, so it needs a notification endpoint of its own.\n\n## To be decided\n\n- the address of the endpoint\n- how the signature is verified\n",
            "BLIK potwierdza płatność innym kanałem niż karty, więc potrzebuje własnego punktu odbioru powiadomień.\n\n## Do ustalenia\n\n- adres punktu odbioru\n- sposób weryfikacji podpisu\n",
        ),
    },
    Page {
        title: Text::new("Card gateway: sandbox", "Bramka kartowa: sandbox"),
        parent: Some(Text::new("Payments", "Płatności")),
        author: 1,
        day: -5,
        body: Text::new(
            "How to test a card payment (WEB-9) without real money.\n\n## Access\n\nThe sandbox keys are in the team's password manager, in the vault *Store / gateway*. We do not paste them into issues or chat.\n\n## Test cards\n\n| Number | Result |\n|---|---|\n| 4111 1111 1111 1111 | payment accepted |\n| 4000 0000 0000 0002 | card declined |\n| 4000 0000 0000 3220 | requires 3-D Secure |\n\nExpiry date: any date in the future. CVC: any three digits.\n\n## Common problems\n\n- The sandbox declines cards **without** 3-D Secure — that is intended, see the comments in WEB-9.\n- An empty cart on the order summary used to end in error 500 (WEB-11); the fix is already deployed.\n\n```bash\ncurl -X POST https://sandbox.gateway.example/v1/payments \\\n  -H \"Authorization: Bearer $GATEWAY_KEY\" \\\n  -d amount=1999 -d currency=GBP\n```\n",
            "Jak testować płatność kartą (WEB-9) bez prawdziwych pieniędzy.\n\n## Dostęp\n\nKlucze do sandboxa są w menedżerze haseł zespołu, w sejfie *Sklep / bramka*. Nie wklejamy ich do zgłoszeń ani na czat.\n\n## Karty testowe\n\n| Numer | Wynik |\n|---|---|\n| 4111 1111 1111 1111 | płatność przyjęta |\n| 4000 0000 0000 0002 | karta odrzucona |\n| 4000 0000 0000 3220 | wymaga 3-D Secure |\n\nData ważności: dowolna przyszła. Kod CVC: dowolne trzy cyfry.\n\n## Częste problemy\n\n- Sandbox odrzuca karty **bez** 3-D Secure — to zamierzone, zob. komentarze w WEB-9.\n- Pusty koszyk w podsumowaniu kończył się błędem 500 (WEB-11); poprawka jest już wdrożona.\n\n```bash\ncurl -X POST https://sandbox.bramka.example/v1/payments \\\n  -H \"Authorization: Bearer $BRAMKA_KLUCZ\" \\\n  -d amount=1999 -d currency=PLN\n```\n",
        ),
    },
    Page {
        title: Text::new("Decisions", "Decyzje"),
        parent: None,
        author: 0,
        day: -26,
        body: Text::new(
            "The register of architecture decisions (ADRs). One page is one decision: the context, the choice and its consequences. We do not edit a decision after the fact — when it stops applying, we add a new one and mark the old one as superseded.\n\n| No. | Decision | Status |\n|---|---|---|\n| 001 | [[ADR-001: Sessions in Redis]] | accepted |\n| 002 | [[ADR-002: One gateway for cards and Pay by bank]] | accepted |\n",
            "Rejestr decyzji architektonicznych (ADR). Jedna strona to jedna decyzja: kontekst, wybór i jego skutki. Decyzji nie edytujemy po fakcie — gdy przestaje obowiązywać, dopisujemy nową i oznaczamy starą jako zastąpioną.\n\n| Nr | Decyzja | Status |\n|---|---|---|\n| 001 | [[ADR-001: Sesje w Redis]] | przyjęta |\n| 002 | [[ADR-002: Jedna bramka dla kart i BLIK]] | przyjęta |\n",
        ),
    },
    Page {
        title: Text::new("ADR-001: Sessions in Redis", "ADR-001: Sesje w Redis"),
        parent: Some(Text::new("Decisions", "Decyzje")),
        author: 4,
        day: -11,
        body: Text::new(
            "**Status:** accepted\n**Deciders:** Daniel Foster, Emma Clarke\n\n## Context\n\nSessions live in the main database. Every request of a signed-in customer is one more query, and the session table grows faster than orders do. During promotions it is the first thing that starts to lock.\n\n## Options considered\n\n1. **Leave them in the database** and add a clean-up of expired sessions.\n2. **Redis**, with the key's time to live equal to the session's lifetime.\n3. **Stateless tokens** (JWT) — rejected: they cannot be revoked when a password changes.\n\n## Decision\n\nWe move sessions to Redis. The guest cart goes there together with the session.\n\n## Consequences\n\n- One query fewer per request; the database stops being the bottleneck of sign-in.\n- There is one more component to maintain and monitor.\n- The migration needs a maintenance window — see [[Production deployment]].\n\n## Related issues\n\n- WEB-13\n",
            "**Status:** przyjęta\n**Decydujący:** Tomasz Kowalczyk, Anna Nowak\n\n## Kontekst\n\nSesje leżą w głównej bazie. Każde żądanie zalogowanego klienta to dodatkowe zapytanie, a tabela sesji rośnie szybciej niż zamówienia. Przy promocjach to ona pierwsza zaczyna blokować.\n\n## Rozważane opcje\n\n1. **Zostawić w bazie** i dodać sprzątanie wygasłych sesji.\n2. **Redis** z czasem życia klucza równym czasowi sesji.\n3. **Tokeny bezstanowe** (JWT) — odrzucone: nie da się ich unieważnić przy zmianie hasła.\n\n## Decyzja\n\nPrzenosimy sesje do Redis. Koszyk gościa trafia tam razem z sesją.\n\n## Skutki\n\n- Jedno zapytanie mniej na żądanie; baza przestaje być wąskim gardłem logowania.\n- Dochodzi nowy element do utrzymania i monitorowania.\n- Migracja wymaga okna serwisowego — zob. [[Wdrożenie produkcyjne]].\n\n## Powiązane zgłoszenia\n\n- WEB-13\n",
        ),
    },
    Page {
        title: Text::new("ADR-002: One gateway for cards and Pay by bank", "ADR-002: Jedna bramka dla kart i BLIK"),
        parent: Some(Text::new("Decisions", "Decyzje")),
        author: 1,
        day: -12,
        body: Text::new(
            "**Status:** accepted\n**Deciders:** Emma Clarke, James Bennett\n\n## Context\n\nCards (WEB-9) and Pay by bank (WEB-10) can be connected through two providers or through one. Two providers mean a lower fee for Pay by bank, but two integrations, two dashboards and two kinds of settlement.\n\n## Decision\n\nWe connect both methods through one gateway.\n\n## Consequences\n\n- One notification format and one place to check the status of a payment.\n- The fee for Pay by bank is higher by about 0.2 percentage points; we come back to this when Pay by bank exceeds 40% of payments.\n- Despite the shared gateway, Pay by bank has a notification channel of its own: [[Pay by bank — notification channel]].\n",
            "**Status:** przyjęta\n**Decydujący:** Anna Nowak, Piotr Zieliński\n\n## Kontekst\n\nKarty (WEB-9) i BLIK (WEB-10) można podłączyć u dwóch dostawców albo u jednego. Dwóch dostawców to niższa prowizja za BLIK, ale dwie integracje, dwa panele i dwa rodzaje rozliczeń.\n\n## Decyzja\n\nObie metody podłączamy przez jedną bramkę.\n\n## Skutki\n\n- Jeden format powiadomień i jedno miejsce do sprawdzania statusu płatności.\n- Prowizja za BLIK wyższa o ok. 0,2 pkt proc.; wracamy do tematu, gdy BLIK przekroczy 40% płatności.\n- BLIK mimo wspólnej bramki ma osobny kanał powiadomień: [[BLIK — kanał powiadomień]].\n",
        ),
    },
    Page {
        title: Text::new("Meetings", "Spotkania"),
        parent: None,
        author: 0,
        day: -21,
        body: Text::new(
            "Notes from the team's meetings. We write action items as a checklist — any line can be turned into an issue with one click.\n",
            "Notatki ze spotkań zespołu. Zadania zapisujemy jako listę do odhaczenia — z każdej linii można jednym kliknięciem zrobić zgłoszenie.\n",
        ),
    },
    Page {
        title: Text::new("Sprint 1 retrospective", "Retrospektywa sprintu 1"),
        parent: Some(Text::new("Meetings", "Spotkania")),
        author: 3,
        day: -6,
        body: Text::new(
            "**Sprint:** WEB Sprint 1\n**Present:** the whole team\n\n## What went well\n\n- CI on every PR (WEB-7) caught two regressions before they reached production.\n- The cart and the address form were delivered ahead of schedule.\n\n## What went worse\n\n- The cart discount bug (WEB-6) only surfaced with a customer — a test of the price after a discount was missing.\n- Code reviews waited two days each.\n\n## Agreed\n\n- [x] We start a code review no later than the next working day\n- [ ] Add a test of the price of an item on promotion\n- [ ] Set up a review rota for the holiday season\n",
            "**Sprint:** WEB Sprint 1\n**Obecni:** cały zespół\n\n## Co poszło dobrze\n\n- CI na każdym PR (WEB-7) złapało dwie regresje, zanim trafiły na produkcję.\n- Koszyk i formularz adresu dowiezione przed terminem.\n\n## Co poszło gorzej\n\n- Błąd z rabatem w koszyku (WEB-6) wyszedł dopiero u klienta — zabrakło testu ceny po rabacie.\n- Przeglądy kodu czekały po dwa dni.\n\n## Ustalenia\n\n- [x] Przegląd kodu zaczynamy najpóźniej następnego dnia roboczego\n- [ ] Dopisać test ceny pozycji objętej promocją\n- [ ] Ustalić dyżur do przeglądów na czas urlopów\n",
        ),
    },
    Page {
        title: Text::new("Sprint 2 planning — notes", "Planowanie sprintu 2 — notatki"),
        parent: Some(Text::new("Meetings", "Spotkania")),
        author: 0,
        day: -5,
        body: Text::new(
            "**Sprint goal:** online payments and the basics of the customer account.\n**Present:** Emma, James, Olivia, Daniel\n\n## Agreed\n\n- Card payment (WEB-9) goes first; Pay by bank (WEB-10) starts once the card form is ready.\n- We extend the end-to-end tests of the checkout flow (WEB-17) to both payment methods only after the webhook.\n- The session migration (WEB-13) waits for a maintenance window.\n\n## Action items\n\n- [ ] Order production API keys from the gateway provider\n- [ ] Prepare test data: an order with a discount and free delivery\n- [ ] Describe rolling back the session migration in the runbook\n- [x] Agree the date of the maintenance window with customer support\n\nTechnical details: [[Payments]].\n",
            "**Cel sprintu:** płatności online i podstawy konta klienta.\n**Obecni:** Anna, Piotr, Maria, Tomasz\n\n## Ustalenia\n\n- Płatność kartą (WEB-9) idzie pierwsza; BLIK (WEB-10) startuje, gdy formularz karty będzie gotowy.\n- Testy e2e ścieżki zakupowej (WEB-17) rozszerzamy o obie metody płatności dopiero po webhooku.\n- Migracja sesji (WEB-13) czeka na okno serwisowe.\n\n## Zadania\n\n- [ ] Zamówić u dostawcy bramki produkcyjne klucze API\n- [ ] Przygotować dane testowe: zamówienie z rabatem i darmową dostawą\n- [ ] Opisać w runbooku wycofanie migracji sesji\n- [x] Ustalić termin okna serwisowego z działem obsługi\n\nSzczegóły techniczne: [[Płatności]].\n",
        ),
    },
    Page {
        title: Text::new("Production deployment", "Wdrożenie produkcyjne"),
        parent: None,
        author: 4,
        day: -19,
        body: Text::new(
            "The runbook for deploying the store to production. The person on call deploys; a second person is reachable by phone.\n\n## When\n\nRegular deployments: Monday to Thursday, until 3 pm. Changes that need downtime (e.g. the session migration, WEB-13): in the maintenance window, Thursday after 10 pm.\n\n## Before you start\n\n- [ ] All tests on the main branch are green\n- [ ] Database schema changes are backward compatible\n- [ ] The deployment is announced on the #store-deploys channel\n\n## Steps\n\n1. Tag the release: `git tag v1.x.y && git push --tags`.\n2. Wait for the pipeline to build the images and deploy them to the staging environment.\n3. Walk through the checkout flow on staging (cart → address → payment with a test card, see [[Card gateway: sandbox]]).\n4. Approve the production deployment in the pipeline.\n5. Watch for 15 minutes: the rate of 5xx errors, the response time of the order summary, the webhook queue.\n\n## Rollback\n\n```bash\n./deploy rollback --to previous\n```\n\nA rollback does not undo database migrations. If the release contained a migration, read its description first.\n\n## After the deployment\n\nPost the result on the channel and close the issues of the release. Architecture background: [[Store architecture]].\n",
            "Runbook wdrożenia sklepu na produkcję. Wdraża osoba z dyżuru; druga osoba jest pod telefonem.\n\n## Kiedy\n\nZwykłe wdrożenia: od poniedziałku do czwartku, do 15:00. Zmiany wymagające przerwy (np. migracja sesji, WEB-13): w oknie serwisowym, czwartek po 22:00.\n\n## Zanim zaczniesz\n\n- [ ] Wszystkie testy na gałęzi głównej są zielone\n- [ ] Zmiany w schemacie bazy są zgodne wstecz\n- [ ] Na kanale #sklep-wdrozenia jest zapowiedź\n\n## Kroki\n\n1. Oznacz wydanie: `git tag v1.x.y && git push --tags`.\n2. Poczekaj, aż potok zbuduje obrazy i wdroży je na środowisko testowe.\n3. Przejdź ścieżkę zakupową na środowisku testowym (koszyk → adres → płatność kartą testową, zob. [[Bramka kartowa: sandbox]]).\n4. Zatwierdź wdrożenie produkcyjne w potoku.\n5. Obserwuj przez 15 minut: odsetek błędów 5xx, czas odpowiedzi podsumowania, kolejkę webhooków.\n\n## Wycofanie\n\n```bash\n./deploy rollback --to poprzednie\n```\n\nWycofanie nie cofa migracji bazy. Jeśli wydanie zawierało migrację, najpierw przeczytaj jej opis.\n\n## Po wdrożeniu\n\nDopisz wynik na kanale i zamknij zgłoszenia z wydania. Tło architektoniczne: [[Architektura sklepu]].\n",
        ),
    },
];

/// A later edit of a page: the day, what the author said about it, and the whole new body.
struct Revision {
    day: i64,
    message: Text,
    body: Text,
}

/// The page on the bank payment's notification channel grew over two edits.
const CHANNEL_PAGE: Text = Text::new("Pay by bank — notification channel", "BLIK — kanał powiadomień");
const CHANNEL_REVISIONS: &[Revision] = &[
    Revision {
        day: -3,
        message: Text::new("Endpoint address and signature verification", "Adres punktu odbioru i weryfikacja podpisu"),
        body: Text::new(
            "Pay by bank confirms a payment over a different channel than cards, so it needs a notification endpoint of its own (WEB-10).\n\n## Endpoint\n\n`POST /webhooks/bank` — separate from `/webhooks/cards`, because Pay by bank notifications come from a different pool of addresses and carry a different signature.\n\n## Verification\n\nThe `X-Bank-Signature` header holds an HMAC-SHA256 of the request body. A notification without a valid signature is rejected with code 401 and **not** stored.\n\n## To be decided\n\n- what we do with a notification that arrives twice\n",
            "BLIK potwierdza płatność innym kanałem niż karty, więc potrzebuje własnego punktu odbioru powiadomień (WEB-10).\n\n## Punkt odbioru\n\n`POST /webhooks/blik` — osobny od `/webhooks/karty`, bo powiadomienia BLIK przychodzą z innej puli adresów i mają inny podpis.\n\n## Weryfikacja\n\nNagłówek `X-Blik-Signature` zawiera HMAC-SHA256 treści żądania. Powiadomienie bez poprawnego podpisu odrzucamy z kodem 401 i **nie** zapisujemy.\n\n## Do ustalenia\n\n- co robimy z powiadomieniem, które przyszło dwa razy\n",
        ),
    },
    Revision {
        day: -2,
        message: Text::new("Retries and the time limit", "Powtórzenia i limit czasu"),
        body: Text::new(
            "Pay by bank confirms a payment over a different channel than cards, so it needs a notification endpoint of its own (WEB-10). The decision on the shared gateway: [[ADR-002: One gateway for cards and Pay by bank]].\n\n## Endpoint\n\n`POST /webhooks/bank` — separate from `/webhooks/cards`, because Pay by bank notifications come from a different pool of addresses and carry a different signature.\n\n## Verification\n\nThe `X-Bank-Signature` header holds an HMAC-SHA256 of the request body. A notification without a valid signature is rejected with code 401 and **not** stored.\n\n## Retries\n\nThe gateway repeats a notification until it gets through, even for a whole day. Each one carries a transaction id; a second notification about the same transaction is acknowledged with code 200 and changes nothing.\n\n## Time limit\n\nThe customer has 10 minutes to approve the payment in their banking app. If no confirmation arrives in that time, the order goes back to the choice of payment method, and the customer sees a message with a *Try again* button.\n\n## Testing\n\nIn the sandbox the bank *Test Bank (approve)* always goes through, and *Test Bank (decline)* is always declined. The rest is as in [[Card gateway: sandbox]].\n",
            "BLIK potwierdza płatność innym kanałem niż karty, więc potrzebuje własnego punktu odbioru powiadomień (WEB-10). Decyzja o wspólnej bramce: [[ADR-002: Jedna bramka dla kart i BLIK]].\n\n## Punkt odbioru\n\n`POST /webhooks/blik` — osobny od `/webhooks/karty`, bo powiadomienia BLIK przychodzą z innej puli adresów i mają inny podpis.\n\n## Weryfikacja\n\nNagłówek `X-Blik-Signature` zawiera HMAC-SHA256 treści żądania. Powiadomienie bez poprawnego podpisu odrzucamy z kodem 401 i **nie** zapisujemy.\n\n## Powtórzenia\n\nBramka ponawia powiadomienie do skutku, nawet przez dobę. Każde ma identyfikator transakcji; drugie powiadomienie o tej samej transakcji potwierdzamy kodem 200 i nic nie zmieniamy.\n\n## Limit czasu\n\nKod BLIK jest ważny 2 minuty. Jeśli w tym czasie nie przyjdzie potwierdzenie, zamówienie wraca do wyboru metody płatności, a klient widzi komunikat z przyciskiem *Spróbuj ponownie*.\n\n## Testowanie\n\nW sandboxie kod `777 123` zawsze przechodzi, a `777 000` zawsze jest odrzucany. Reszta jak w [[Bramka kartowa: sandbox]].\n",
        ),
    },
];

const TEAM_PAGES: &[Page] = &[
    Page {
        title: Text::new("How the team works", "Zasady pracy zespołu"),
        parent: None,
        author: 0,
        day: -29,
        body: Text::new(
            "Briefly, how we work.\n\n## Issues\n\n- Every change in the code has an issue; we put the issue key in the branch name and in the PR title.\n- An issue *in progress* has one person assigned. Whoever stops working on it removes the assignment.\n\n## Code reviews\n\n- We start a review no later than the next working day.\n- We split remarks into *blocking* and *to consider* — the author knows what has to be fixed.\n\n## Meetings\n\n- Daily: 9:30 am, 15 minutes at most.\n- Planning and retrospective: every two weeks; the notes go to the project's knowledge base.\n\n## Documentation\n\nWe record decisions as ADRs and procedures as runbooks. If someone asks about something a second time, the answer belongs on a page.\n",
            "Krótko o tym, jak pracujemy.\n\n## Zgłoszenia\n\n- Każda zmiana w kodzie ma zgłoszenie; klucz zgłoszenia wpisujemy w nazwie gałęzi i tytule PR.\n- Zgłoszenie *w toku* ma jedną osobę przypisaną. Kto przestaje nad nim pracować, zdejmuje przypisanie.\n\n## Przeglądy kodu\n\n- Przegląd zaczynamy najpóźniej następnego dnia roboczego.\n- Uwagi dzielimy na *blokujące* i *do rozważenia* — autor wie, co musi poprawić.\n\n## Spotkania\n\n- Codzienne: 9:30, najwyżej 15 minut.\n- Planowanie i retrospektywa: co dwa tygodnie, notatki trafiają do bazy wiedzy projektu.\n\n## Dokumentacja\n\nDecyzje zapisujemy jako ADR-y, procedury jako runbooki. Jeśli ktoś pyta o coś drugi raz, odpowiedź powinna trafić na stronę.\n",
        ),
    },
    Page {
        title: Text::new("Onboarding a new team member", "Onboarding nowej osoby"),
        parent: None,
        author: 3,
        day: -15,
        body: Text::new(
            "A checklist for the first week. The new person's buddy ticks it off together with them.\n\n## Day one\n\n- [ ] An account in team-hub, access to the repositories and the password manager\n- [ ] Reading: [[How the team works]]\n- [ ] Reading: [[WEB:Store architecture]]\n\n## First week\n\n- [ ] Run the store locally and walk through the checkout flow\n- [ ] A first issue with the label *good first issue*\n- [ ] Take part in a code review as the second reviewer\n- [ ] Assist with a deployment following the runbook [[WEB:Production deployment]]\n",
            "Lista na pierwszy tydzień. Opiekun nowej osoby odhacza razem z nią.\n\n## Dzień pierwszy\n\n- [ ] Konto w team-hub, dostęp do repozytoriów i menedżera haseł\n- [ ] Lektura: [[Zasady pracy zespołu]]\n- [ ] Lektura: [[WEB:Architektura sklepu]]\n\n## Pierwszy tydzień\n\n- [ ] Uruchomić sklep lokalnie i przejść ścieżkę zakupową\n- [ ] Pierwsze zgłoszenie z etykietą *dobre na start*\n- [ ] Udział w przeglądzie kodu jako druga osoba\n- [ ] Asysta przy wdrożeniu według runbooka [[WEB:Wdrożenie produkcyjne]]\n",
        ),
    },
];

fn at(day: i64, hour: i64) -> String {
    let instant = Utc::now() + Duration::days(day) - Duration::hours(hour % 7) - Duration::minutes(20);

    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn page_id(app: &App, space: &SpaceRecord, title: &str) -> ApiResult<Id> {
    app.read(|connection| query_one(connection, "SELECT id FROM wiki_page WHERE space_id = ?1 AND title = ?2", (space.id, title), |row| row.get(0)))?
        .ok_or_else(|| ApiError::internal(format!("demo data refers to the page \"{title}\", which does not exist")))
}

fn seed_pages(app: &App, language: DemoLanguage, space: &SpaceRecord, pages: &[Page], people: &[Id]) -> ApiResult<()> {
    let mut ordered: Vec<(usize, &Page)> = pages.iter().enumerate().collect();
    ordered.sort_by_key(|(_, page)| page.day);

    for (index, page) in ordered {
        let parent_id = page.parent.map(|title| page_id(app, space, title.pick(language))).transpose()?;
        let author = people[page.author % people.len()];

        app.write_at(Actor::user(author), at(page.day, index as i64), |tx| {
            pages::create_page(tx, space, page.title.pick(language), page.body.pick(language), parent_id)
        })?;
    }

    // Pages were written in the order of their dates; put each level back in the order of the list.
    for (position, page) in pages.iter().enumerate() {
        let id = page_id(app, space, page.title.pick(language))?;

        app.db.write(|tx| Ok(tx.execute("UPDATE wiki_page SET position = ?1 WHERE id = ?2", (position as i64, id))?))?;
    }

    Ok(())
}

/// A space of the demo: its key, its name and what it is about.
struct Space {
    key: &'static str,
    name: Text,
    description: Text,
}

const WEB_SPACE: Space = Space {
    key: "WEB",
    name: Text::new("Store — knowledge base", "Sklep — baza wiedzy"),
    description: Text::new(
        "Architecture, payments, decisions and procedures of the online store.",
        "Architektura, płatności, decyzje i procedury sklepu internetowego.",
    ),
};

const TEAM_SPACE: Space = Space {
    key: "TEAM",
    name: Text::new("Team", "Zespół"),
    description: Text::new("How we work: rules, meetings, onboarding.", "Jak pracujemy: zasady, spotkania, wdrażanie nowych osób."),
};

fn seed_space(app: &App, language: DemoLanguage, owner: Id, space: &Space, project_id: Option<Id>, when: String) -> ApiResult<SpaceRecord> {
    app.write_at(Actor::user(owner), when, |tx| {
        spaces::create_space(
            tx,
            &NewSpace {
                key: space.key,
                name: space.name.pick(language),
                description: space.description.pick(language),
                project_id,
                source: "local",
                source_ref: None,
            },
        )
    })
}

pub fn seed_demo(app: &App, language: DemoLanguage) -> ApiResult<()> {
    let (people, project_id, seeded) = app.read(|connection| {
        let people: Vec<Id> = query_all(connection, "SELECT id FROM user WHERE active = 1 ORDER BY id LIMIT 5", [], |row| row.get(0))?;
        let project_id: Option<Id> = query_one(connection, "SELECT id FROM project WHERE key = 'WEB'", [], |row| row.get(0))?;
        let seeded = spaces::space_by_key(connection, WEB_SPACE.key)?.is_some() || spaces::space_by_key(connection, TEAM_SPACE.key)?.is_some();

        Ok((people, project_id, seeded))
    })?;
    let (Some(owner), Some(project_id), false) = (people.first().copied(), project_id, seeded) else {
        return Ok(());
    };

    let web = seed_space(app, language, owner, &WEB_SPACE, Some(project_id), at(-28, 0))?;
    let team = seed_space(app, language, owner, &TEAM_SPACE, None, at(-29, 1))?;

    seed_pages(app, language, &web, WEB_PAGES, &people)?;
    seed_pages(app, language, &team, TEAM_PAGES, &people)?;

    let channel = page_id(app, &web, CHANNEL_PAGE.pick(language))?;
    let author = people[2 % people.len()];

    for (index, revision) in CHANNEL_REVISIONS.iter().enumerate() {
        app.write_at(Actor::user(author), at(revision.day, 3), |tx| {
            let page = pages::load_page(tx, channel)?;
            let edit = PageEdit {
                title: None,
                body: Some(revision.body.pick(language)),
                base_version: index as i64 + 1,
                message: revision.message.pick(language),
            };

            pages::update_page(tx, &web, &page, &edit)
        })?;
    }

    Ok(())
}
