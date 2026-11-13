//! Example pages for a demo instance: the knowledge base of the demo shop project and a
//! small team space, written over the same weeks as the demo issues.

use chrono::{Duration, SecondsFormat, Utc};
use hub_app::db::{query_all, query_one};
use hub_app::{Actor, ApiError, ApiResult, App};
use hub_core::Id;

use crate::pages::{self, PageEdit};
use crate::spaces::{self, NewSpace, SpaceRecord};

struct Page {
    title: &'static str,
    parent: Option<&'static str>,
    /// Index into the demo people; 0 is whoever set the instance up.
    author: usize,
    day: i64,
    body: &'static str,
}

const WEB_PAGES: &[Page] = &[
    Page {
        title: "Architektura sklepu",
        parent: None,
        author: 4,
        day: -27,
        body: "Sklep składa się z trzech części: aplikacji przeglądarkowej, API zamówień i usług pomocniczych (płatności, e-maile, wyszukiwarka).\n\n## Komponenty\n\n| Komponent | Technologia | Odpowiada za |\n|---|---|---|\n| Frontend | React, Vite | katalog, koszyk, konto klienta |\n| API | Rust, PostgreSQL | zamówienia, ceny, stany magazynowe |\n| Płatności | bramka zewnętrzna | karty i BLIK, zob. [[Płatności]] |\n| Sesje | Redis (w trakcie migracji) | logowanie, koszyk gościa |\n\n## Przepływ zamówienia\n\n1. Klient dodaje produkty do koszyka; koszyk gościa żyje w sesji.\n2. W podsumowaniu API przelicza ceny i rabaty po swojej stronie.\n3. Po wyborze metody płatności zamówienie dostaje status *oczekuje na płatność*.\n4. Bramka potwierdza płatność webhookiem; dopiero wtedy wysyłamy e-mail z potwierdzeniem.\n\n## Sesje\n\nSesje trzymamy dziś w bazie. Przeniesienie ich do Redis opisuje [[ADR-001: Sesje w Redis]], a pracę śledzi WEB-13.\n\n## Zobacz też\n\n- [[Decyzje]] — dlaczego jest tak, a nie inaczej\n- [[Wdrożenie produkcyjne]] — jak to wszystko trafia na produkcję\n",
    },
    Page {
        title: "Płatności",
        parent: None,
        author: 1,
        day: -13,
        body: "Wszystkie płatności online idą przez jedną bramkę (uzasadnienie: [[ADR-002: Jedna bramka dla kart i BLIK]]).\n\n| Metoda | Zgłoszenie | Stan |\n|---|---|---|\n| Karta (3-D Secure) | WEB-9 | w przeglądzie |\n| BLIK | WEB-10 | w toku |\n| Za pobraniem | — | działa od wersji 1.0 |\n\n## Zasady\n\n- Zamówienie zmienia status **wyłącznie** po webhooku z bramki, nigdy po powrocie klienta ze strony płatności.\n- Każde powiadomienie z bramki zapisujemy w całości, zanim je przetworzymy.\n- Kwoty liczymy w groszach, bez liczb zmiennoprzecinkowych.\n\n## Podstrony\n\n- [[BLIK — kanał powiadomień]]\n- [[Bramka kartowa: sandbox]]\n",
    },
    Page {
        title: "BLIK — kanał powiadomień",
        parent: Some("Płatności"),
        author: 2,
        day: -6,
        body: "BLIK potwierdza płatność innym kanałem niż karty, więc potrzebuje własnego punktu odbioru powiadomień.\n\n## Do ustalenia\n\n- adres punktu odbioru\n- sposób weryfikacji podpisu\n",
    },
    Page {
        title: "Bramka kartowa: sandbox",
        parent: Some("Płatności"),
        author: 1,
        day: -5,
        body: "Jak testować płatność kartą (WEB-9) bez prawdziwych pieniędzy.\n\n## Dostęp\n\nKlucze do sandboxa są w menedżerze haseł zespołu, w sejfie *Sklep / bramka*. Nie wklejamy ich do zgłoszeń ani na czat.\n\n## Karty testowe\n\n| Numer | Wynik |\n|---|---|\n| 4111 1111 1111 1111 | płatność przyjęta |\n| 4000 0000 0000 0002 | karta odrzucona |\n| 4000 0000 0000 3220 | wymaga 3-D Secure |\n\nData ważności: dowolna przyszła. Kod CVC: dowolne trzy cyfry.\n\n## Częste problemy\n\n- Sandbox odrzuca karty **bez** 3-D Secure — to zamierzone, zob. komentarze w WEB-9.\n- Pusty koszyk w podsumowaniu kończył się błędem 500 (WEB-11); poprawka jest już wdrożona.\n\n```bash\ncurl -X POST https://sandbox.bramka.example/v1/payments \\\n  -H \"Authorization: Bearer $BRAMKA_KLUCZ\" \\\n  -d amount=1999 -d currency=PLN\n```\n",
    },
    Page {
        title: "Decyzje",
        parent: None,
        author: 0,
        day: -26,
        body: "Rejestr decyzji architektonicznych (ADR). Jedna strona to jedna decyzja: kontekst, wybór i jego skutki. Decyzji nie edytujemy po fakcie — gdy przestaje obowiązywać, dopisujemy nową i oznaczamy starą jako zastąpioną.\n\n| Nr | Decyzja | Status |\n|---|---|---|\n| 001 | [[ADR-001: Sesje w Redis]] | przyjęta |\n| 002 | [[ADR-002: Jedna bramka dla kart i BLIK]] | przyjęta |\n",
    },
    Page {
        title: "ADR-001: Sesje w Redis",
        parent: Some("Decyzje"),
        author: 4,
        day: -11,
        body: "**Status:** przyjęta\n**Decydujący:** Tomasz Kowalczyk, Anna Nowak\n\n## Kontekst\n\nSesje leżą w głównej bazie. Każde żądanie zalogowanego klienta to dodatkowe zapytanie, a tabela sesji rośnie szybciej niż zamówienia. Przy promocjach to ona pierwsza zaczyna blokować.\n\n## Rozważane opcje\n\n1. **Zostawić w bazie** i dodać sprzątanie wygasłych sesji.\n2. **Redis** z czasem życia klucza równym czasowi sesji.\n3. **Tokeny bezstanowe** (JWT) — odrzucone: nie da się ich unieważnić przy zmianie hasła.\n\n## Decyzja\n\nPrzenosimy sesje do Redis. Koszyk gościa trafia tam razem z sesją.\n\n## Skutki\n\n- Jedno zapytanie mniej na żądanie; baza przestaje być wąskim gardłem logowania.\n- Dochodzi nowy element do utrzymania i monitorowania.\n- Migracja wymaga okna serwisowego — zob. [[Wdrożenie produkcyjne]].\n\n## Powiązane zgłoszenia\n\n- WEB-13\n",
    },
    Page {
        title: "ADR-002: Jedna bramka dla kart i BLIK",
        parent: Some("Decyzje"),
        author: 1,
        day: -12,
        body: "**Status:** przyjęta\n**Decydujący:** Anna Nowak, Piotr Zieliński\n\n## Kontekst\n\nKarty (WEB-9) i BLIK (WEB-10) można podłączyć u dwóch dostawców albo u jednego. Dwóch dostawców to niższa prowizja za BLIK, ale dwie integracje, dwa panele i dwa rodzaje rozliczeń.\n\n## Decyzja\n\nObie metody podłączamy przez jedną bramkę.\n\n## Skutki\n\n- Jeden format powiadomień i jedno miejsce do sprawdzania statusu płatności.\n- Prowizja za BLIK wyższa o ok. 0,2 pkt proc.; wracamy do tematu, gdy BLIK przekroczy 40% płatności.\n- BLIK mimo wspólnej bramki ma osobny kanał powiadomień: [[BLIK — kanał powiadomień]].\n",
    },
    Page {
        title: "Spotkania",
        parent: None,
        author: 0,
        day: -21,
        body: "Notatki ze spotkań zespołu. Zadania zapisujemy jako listę do odhaczenia — z każdej linii można jednym kliknięciem zrobić zgłoszenie.\n",
    },
    Page {
        title: "Retrospektywa sprintu 1",
        parent: Some("Spotkania"),
        author: 3,
        day: -6,
        body: "**Sprint:** WEB Sprint 1\n**Obecni:** cały zespół\n\n## Co poszło dobrze\n\n- CI na każdym PR (WEB-7) złapało dwie regresje, zanim trafiły na produkcję.\n- Koszyk i formularz adresu dowiezione przed terminem.\n\n## Co poszło gorzej\n\n- Błąd z rabatem w koszyku (WEB-6) wyszedł dopiero u klienta — zabrakło testu ceny po rabacie.\n- Przeglądy kodu czekały po dwa dni.\n\n## Ustalenia\n\n- [x] Przegląd kodu zaczynamy najpóźniej następnego dnia roboczego\n- [ ] Dopisać test ceny pozycji objętej promocją\n- [ ] Ustalić dyżur do przeglądów na czas urlopów\n",
    },
    Page {
        title: "Planowanie sprintu 2 — notatki",
        parent: Some("Spotkania"),
        author: 0,
        day: -5,
        body: "**Cel sprintu:** płatności online i podstawy konta klienta.\n**Obecni:** Anna, Piotr, Maria, Tomasz\n\n## Ustalenia\n\n- Płatność kartą (WEB-9) idzie pierwsza; BLIK (WEB-10) startuje, gdy formularz karty będzie gotowy.\n- Testy e2e ścieżki zakupowej (WEB-17) rozszerzamy o obie metody płatności dopiero po webhooku.\n- Migracja sesji (WEB-13) czeka na okno serwisowe.\n\n## Zadania\n\n- [ ] Zamówić u dostawcy bramki produkcyjne klucze API\n- [ ] Przygotować dane testowe: zamówienie z rabatem i darmową dostawą\n- [ ] Opisać w runbooku wycofanie migracji sesji\n- [x] Ustalić termin okna serwisowego z działem obsługi\n\nSzczegóły techniczne: [[Płatności]].\n",
    },
    Page {
        title: "Wdrożenie produkcyjne",
        parent: None,
        author: 4,
        day: -19,
        body: "Runbook wdrożenia sklepu na produkcję. Wdraża osoba z dyżuru; druga osoba jest pod telefonem.\n\n## Kiedy\n\nZwykłe wdrożenia: od poniedziałku do czwartku, do 15:00. Zmiany wymagające przerwy (np. migracja sesji, WEB-13): w oknie serwisowym, czwartek po 22:00.\n\n## Zanim zaczniesz\n\n- [ ] Wszystkie testy na gałęzi głównej są zielone\n- [ ] Zmiany w schemacie bazy są zgodne wstecz\n- [ ] Na kanale #sklep-wdrozenia jest zapowiedź\n\n## Kroki\n\n1. Oznacz wydanie: `git tag v1.x.y && git push --tags`.\n2. Poczekaj, aż potok zbuduje obrazy i wdroży je na środowisko testowe.\n3. Przejdź ścieżkę zakupową na środowisku testowym (koszyk → adres → płatność kartą testową, zob. [[Bramka kartowa: sandbox]]).\n4. Zatwierdź wdrożenie produkcyjne w potoku.\n5. Obserwuj przez 15 minut: odsetek błędów 5xx, czas odpowiedzi podsumowania, kolejkę webhooków.\n\n## Wycofanie\n\n```bash\n./deploy rollback --to poprzednie\n```\n\nWycofanie nie cofa migracji bazy. Jeśli wydanie zawierało migrację, najpierw przeczytaj jej opis.\n\n## Po wdrożeniu\n\nDopisz wynik na kanale i zamknij zgłoszenia z wydania. Tło architektoniczne: [[Architektura sklepu]].\n",
    },
];

const BLIK_REVISIONS: &[(i64, &str, &str)] = &[
    (
        -3,
        "Adres punktu odbioru i weryfikacja podpisu",
        "BLIK potwierdza płatność innym kanałem niż karty, więc potrzebuje własnego punktu odbioru powiadomień (WEB-10).\n\n## Punkt odbioru\n\n`POST /webhooks/blik` — osobny od `/webhooks/karty`, bo powiadomienia BLIK przychodzą z innej puli adresów i mają inny podpis.\n\n## Weryfikacja\n\nNagłówek `X-Blik-Signature` zawiera HMAC-SHA256 treści żądania. Powiadomienie bez poprawnego podpisu odrzucamy z kodem 401 i **nie** zapisujemy.\n\n## Do ustalenia\n\n- co robimy z powiadomieniem, które przyszło dwa razy\n",
    ),
    (
        -2,
        "Powtórzenia i limit czasu",
        "BLIK potwierdza płatność innym kanałem niż karty, więc potrzebuje własnego punktu odbioru powiadomień (WEB-10). Decyzja o wspólnej bramce: [[ADR-002: Jedna bramka dla kart i BLIK]].\n\n## Punkt odbioru\n\n`POST /webhooks/blik` — osobny od `/webhooks/karty`, bo powiadomienia BLIK przychodzą z innej puli adresów i mają inny podpis.\n\n## Weryfikacja\n\nNagłówek `X-Blik-Signature` zawiera HMAC-SHA256 treści żądania. Powiadomienie bez poprawnego podpisu odrzucamy z kodem 401 i **nie** zapisujemy.\n\n## Powtórzenia\n\nBramka ponawia powiadomienie do skutku, nawet przez dobę. Każde ma identyfikator transakcji; drugie powiadomienie o tej samej transakcji potwierdzamy kodem 200 i nic nie zmieniamy.\n\n## Limit czasu\n\nKod BLIK jest ważny 2 minuty. Jeśli w tym czasie nie przyjdzie potwierdzenie, zamówienie wraca do wyboru metody płatności, a klient widzi komunikat z przyciskiem *Spróbuj ponownie*.\n\n## Testowanie\n\nW sandboxie kod `777 123` zawsze przechodzi, a `777 000` zawsze jest odrzucany. Reszta jak w [[Bramka kartowa: sandbox]].\n",
    ),
];

const TEAM_PAGES: &[Page] = &[
    Page {
        title: "Zasady pracy zespołu",
        parent: None,
        author: 0,
        day: -29,
        body: "Krótko o tym, jak pracujemy.\n\n## Zgłoszenia\n\n- Każda zmiana w kodzie ma zgłoszenie; klucz zgłoszenia wpisujemy w nazwie gałęzi i tytule PR.\n- Zgłoszenie *w toku* ma jedną osobę przypisaną. Kto przestaje nad nim pracować, zdejmuje przypisanie.\n\n## Przeglądy kodu\n\n- Przegląd zaczynamy najpóźniej następnego dnia roboczego.\n- Uwagi dzielimy na *blokujące* i *do rozważenia* — autor wie, co musi poprawić.\n\n## Spotkania\n\n- Codzienne: 9:30, najwyżej 15 minut.\n- Planowanie i retrospektywa: co dwa tygodnie, notatki trafiają do bazy wiedzy projektu.\n\n## Dokumentacja\n\nDecyzje zapisujemy jako ADR-y, procedury jako runbooki. Jeśli ktoś pyta o coś drugi raz, odpowiedź powinna trafić na stronę.\n",
    },
    Page {
        title: "Onboarding nowej osoby",
        parent: None,
        author: 3,
        day: -15,
        body: "Lista na pierwszy tydzień. Opiekun nowej osoby odhacza razem z nią.\n\n## Dzień pierwszy\n\n- [ ] Konto w team-hub, dostęp do repozytoriów i menedżera haseł\n- [ ] Lektura: [[Zasady pracy zespołu]]\n- [ ] Lektura: [[WEB:Architektura sklepu]]\n\n## Pierwszy tydzień\n\n- [ ] Uruchomić sklep lokalnie i przejść ścieżkę zakupową\n- [ ] Pierwsze zgłoszenie z etykietą *dobre na start*\n- [ ] Udział w przeglądzie kodu jako druga osoba\n- [ ] Asysta przy wdrożeniu według runbooka [[WEB:Wdrożenie produkcyjne]]\n",
    },
];

fn at(day: i64, hour: i64) -> String {
    let instant = Utc::now() + Duration::days(day) - Duration::hours(hour % 7) - Duration::minutes(20);

    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn page_id(app: &App, space: &SpaceRecord, title: &str) -> ApiResult<Id> {
    app.read(|connection| {
        query_one(
            connection,
            "SELECT id FROM wiki_page WHERE space_id = ?1 AND title = ?2",
            (space.id, title),
            |row| row.get(0),
        )
    })?
    .ok_or_else(|| ApiError::internal(format!("demo data refers to the page \"{title}\", which does not exist")))
}

fn seed_pages(app: &App, space: &SpaceRecord, pages: &[Page], people: &[Id]) -> ApiResult<()> {
    let mut ordered: Vec<(usize, &Page)> = pages.iter().enumerate().collect();
    ordered.sort_by_key(|(_, page)| page.day);

    for (index, page) in ordered {
        let parent_id = page.parent.map(|title| page_id(app, space, title)).transpose()?;
        let author = people[page.author % people.len()];

        app.write_at(Actor::user(author), at(page.day, index as i64), |tx| {
            pages::create_page(tx, space, page.title, page.body, parent_id)
        })?;
    }

    // Pages were written in the order of their dates; put each level back in the order of the list.
    for (position, page) in pages.iter().enumerate() {
        let id = page_id(app, space, page.title)?;

        app.db
            .write(|tx| Ok(tx.execute("UPDATE wiki_page SET position = ?1 WHERE id = ?2", (position as i64, id))?))?;
    }

    Ok(())
}

pub fn seed_demo(app: &App) -> ApiResult<()> {
    let (people, project_id, seeded) = app.read(|connection| {
        let people: Vec<Id> = query_all(connection, "SELECT id FROM user WHERE active = 1 ORDER BY id LIMIT 5", [], |row| {
            row.get(0)
        })?;
        let project_id: Option<Id> = query_one(connection, "SELECT id FROM project WHERE key = 'WEB'", [], |row| row.get(0))?;
        let seeded = spaces::space_by_key(connection, "WEB")?.is_some() || spaces::space_by_key(connection, "TEAM")?.is_some();

        Ok((people, project_id, seeded))
    })?;
    let (Some(owner), Some(project_id), false) = (people.first().copied(), project_id, seeded) else {
        return Ok(());
    };

    let web = app.write_at(Actor::user(owner), at(-28, 0), |tx| {
        spaces::create_space(
            tx,
            &NewSpace {
                key: "WEB",
                name: "Sklep — baza wiedzy",
                description: "Architektura, płatności, decyzje i procedury sklepu internetowego.",
                project_id: Some(project_id),
                source: "local",
                source_ref: None,
            },
        )
    })?;
    let team = app.write_at(Actor::user(owner), at(-29, 1), |tx| {
        spaces::create_space(
            tx,
            &NewSpace {
                key: "TEAM",
                name: "Zespół",
                description: "Jak pracujemy: zasady, spotkania, wdrażanie nowych osób.",
                project_id: None,
                source: "local",
                source_ref: None,
            },
        )
    })?;

    seed_pages(app, &web, WEB_PAGES, &people)?;
    seed_pages(app, &team, TEAM_PAGES, &people)?;

    let blik = page_id(app, &web, "BLIK — kanał powiadomień")?;
    let author = people[2 % people.len()];

    for (index, (day, message, body)) in BLIK_REVISIONS.iter().enumerate() {
        app.write_at(Actor::user(author), at(*day, 3), |tx| {
            let page = pages::load_page(tx, blik)?;

            pages::update_page(
                tx,
                &web,
                &page,
                &PageEdit {
                    title: None,
                    body: Some(body),
                    base_version: index as i64 + 1,
                    message,
                },
            )
        })?;
    }

    Ok(())
}
