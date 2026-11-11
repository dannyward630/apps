# Git Tracker

Erfassen Sie numerische Messwerte für Git-Commits unter [git-tracker.delino.io](https://git-tracker.delino.io). Git Tracker ist kostenlos. Er unterstützt öffentliche und private GitHub-Repositorys, einschließlich Pull Requests aus Forks. Verwenden Sie ihn für Dateigrößen, Zeitmessungen, Durchsatz, Anzahlen und andere skalare Messwerte.

## Ein Repository verbinden

1. Melden Sie sich mit Ihrem Delino-Konto an und wählen Sie Ihren persönlichen Space oder den Space Ihrer Organisation aus.
2. Bitten Sie einen Space-Inhaber, Ihnen über die Gruppen des Spaces die Git-Tracker-Berechtigung „View“, „Edit“ oder „Manage“ zu erteilen. „View“ erlaubt Lesezugriffe. „Edit“ erlaubt zusätzlich Uploads und das Abschließen von Ausführungen. „Manage“ erlaubt zusätzlich das Verbinden, die Token-Verwaltung und das dauerhafte Löschen.
3. Installieren Sie die Git Tracker GitHub App für das Repository.
4. Öffnen Sie **Repositorys verwalten**. Geben Sie `owner/repository` und ein GitHub-Zugriffstoken für Ihr verknüpftes GitHub-Konto mit Administratorzugriff auf das Repository ein. Der Dienst überprüft Ihre GitHub-Berechtigung und speichert dieses Zugriffstoken nicht.
5. Verbinden Sie das Repository. Sie benötigen außerdem Owner-/Admin-Berechtigungen für den Delino-Space. Ein Repository kann jeweils nur mit einem Space verbunden sein. Kopieren Sie seine Verbindungs-ID aus der Repository-Verwaltung für Action- und API-Aufrufe. Nach dem Trennen kann ein anderer autorisierter Space es unter einer neuen Verbindungs-ID verbinden; der aufbewahrte Verlauf des alten Spaces bleibt privat.

Messwerte öffentlicher Repositorys sind ohne Anmeldung lesbar. Private Messwerte erfordern eine Space-Mitgliedschaft und Git-Tracker-Leseberechtigung. Der Space-Zugriff gilt für alle verbundenen Repositorys in diesem Space.

Das Trennen oder Entfernen der App stoppt Uploads und macht den aufbewahrten Verlauf privat. Autorisierte Mitglieder können ihn weiterhin lesen. Eine bestätigte Änderung von öffentlich zu privat macht historische Daten privat. Wenn GitHub die aktuelle Sichtbarkeit nicht bestätigen kann, behält Git Tracker die zuletzt bestätigte Sichtbarkeit bei, bis die Synchronisierung erfolgreich ist.

## Ihren Datensatz definieren

Hinterlegen Sie eine YAML-Konfiguration in Ihrem Repository. IDs sind stabil; Namen können sich ändern, ohne dass der Verlauf verloren geht. Alle Spalten einer Zeile müssen zusammen hochgeladen werden.

```yaml
id: file-size
name: JavaScript file sizes
columns:
  - id: original
    name: Original size
    unit: B
    direction: lower
    absoluteThreshold: 1024
  - id: minified
    name: Minified size
    unit: B
    direction: lower
    percentageThreshold: 1
  - id: gzip
    name: Gzipped size
    unit: B
    direction: lower
    absoluteThreshold: 128
    percentageThreshold: 2
```

Laden Sie JSON-Zeilen hoch, zum Beispiel eine Dateigrößentabelle im SWC-Stil:

```json
{
  "rows": [
    { "id": "jquery", "name": "jQuery", "values": { "original": 285314, "minified": 87533, "gzip": 30321 } },
    { "id": "three", "name": "Three.js", "values": { "original": 1321000, "minified": 675000, "gzip": 170000 } }
  ]
}
```

Die Action bewahrt die Dezimalschreibweise aus JSON ohne Umwandlung in binäre Gleitkommazahlen. Direkte Connect-Clients senden Dezimalwerte als Zeichenfolgen, etwa `"9007199254740993.0000000000000001"`. Negative Zahlen und endliche Dezimalzahlen werden unterstützt. Rundungen für die Anzeige ändern niemals die Regressionsentscheidungen.

Unterstützte Einheiten sind `B`, `kB`, `MB`, `GB`, `KiB`, `MiB`, `GiB`; `ns`, `μs`, `ms`, `s`; `count`, `%`, `unitless`; sowie `ops/s`, `B/s`, `kB/s`, `MB/s`, `GB/s`, `KiB/s`, `MiB/s`, `GiB/s`. Kompatible Einheiten werden vor dem Vergleich umgerechnet. Inkompatible Größenarten sind nicht vergleichbar. Beispielsweise wird 1048576 B als 1 MiB angezeigt; exakte Werte bleiben in Tabellen sichtbar.

Die Richtungen sind `lower`, `higher` und `neutral`. Neutrale Spalten erzeugen niemals Regressionen. Schwellenwerte gelten für alle Zeilen in der Spalte. Wenn sowohl absolute als auch prozentuale Schwellenwerte festgelegt sind, gilt das Überschreiten eines der beiden als Regression; Gleichheit nicht. Ohne Schwellenwert werden Änderungen angezeigt, führen aber nicht zu regressionsbedingten Fehlern. Nur Änderungen in die verschlechternde Richtung werden bewertet.

Die prozentuale Änderung ist `(new - baseline) / abs(baseline) * 100`. Bei einem Basiswert von null gibt es keine prozentuale Änderung. Ein absoluter Schwellenwert kann dennoch eine Entscheidung ermöglichen; eine rein prozentuale Regel setzt die Bewertung aus. Vergleiche verwenden die gespeicherte Konfiguration der Zielausführung, nicht eine spätere Konfiguration.

## Die offizielle GitHub Action verwenden

Fixieren Sie `delinoio/apps/actions/git-tracker` auf einen geprüften vollständigen Commit-SHA. Ersetzen Sie die folgenden Platzhalter durch Ihre Git-Tracker-Verbindungs-ID, die Checkout-Revision und die Action-Revision. Gewähren Sie `id-token: write` für OIDC. Ein Repository-Upload-Token ist eine Alternative; speichern Sie es als GitHub-Secret und übergeben Sie `token`.

```yaml
permissions:
  contents: read
  id-token: write
jobs:
  measure:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@<reviewed-checkout-commit-sha>
      - run: ./measure.sh
      - uses: delinoio/apps/actions/git-tracker@<reviewed-action-commit-sha>
        with:
          repository-id: '019b0000-0000-7000-8000-000000000001'
          configuration: measurements.yaml
          measurements: measurements.json
          fail-on-regression: 'false'
  mark-as-done:
    needs: measure
    if: always()
    runs-on: ubuntu-latest
    steps:
      - uses: delinoio/apps/actions/git-tracker@<reviewed-action-commit-sha>
        with:
          operation: complete
          repository-id: '019b0000-0000-7000-8000-000000000001'
```

Unabhängige Jobs teilen sich die standardmäßige Ausführungs-ID `repository_id/run_id/run_attempt` und können überschneidungsfreie Zeilen hochladen. Verwenden Sie für erneute Ausführungen eine neue Ausführungs-ID. Actions-OIDC erfordert die Standardidentität; benutzerdefinierte Ausführungs-IDs sind für Aufrufer mit Repository-Token vorgesehen. Legen Sie bei einem Pull-Request-Job, dessen OIDC-Anmeldedaten zum verbundenen Repository gehören, die Eingabe `baseline` der Action fest, um einen expliziten vollständigen Commit-SHA zu verwenden. OIDC-Uploads aus Forks sind auf den PR-Head beschränkt und können weder die Vergleichsbasis überschreiben noch in den Push-Verlauf des Upstream-Repositorys schreiben.

Jeder Upload-Job bewertet nur seine eigenen hochgeladenen Zeilen. Regressionen werden standardmäßig gemeldet, ohne dass die CI fehlschlägt. Setzen Sie `fail-on-regression: 'true'`, damit dieser Job bei Regressionen fehlschlägt. Fehlende Vergleiche lassen die CI nicht fehlschlagen. Der Abschlussjob meldet lediglich, dass die Messung abgeschlossen ist; er übermittelt weder CI-Erfolg, -Fehlschlag oder -Abbruch noch führt er eine zusammenfassende Freigabeprüfung durch. Ein leerer Abschluss ist gültig, wenn alle Jobs übersprungen wurden.

Bei Netzwerkfehlern und vorübergehenden API-Fehlern werden höchstens drei Wiederholungsversuche unternommen. Standardmäßig schlägt der Job nach den Wiederholungsversuchen fehl. Setzen Sie `warn-on-api-error: 'true'`, um stattdessen zu warnen und den Job erfolgreich abzuschließen. Fehler bei PR-Kommentaren erzeugen nur Warnungen; sie machen gespeicherte Daten nicht ungültig und lassen die CI nicht eigenständig fehlschlagen. Die Action protokolliert einen Detail-Link und schreibt eine Job-Zusammenfassung.

Ein einzelner PR-Kommentar zeigt die kumulierten Ergebnisse der zuletzt registrierten Ausführung für den neuesten Commit des PRs. Ältere Ausführungen bleiben im Webverlauf verfügbar.

## Die Connect API aufrufen

Der API-Ursprung ist `https://git-tracker.api.delino.io`. Connect-JSON-Anfragen verwenden `Content-Type: application/json` und `Connect-Protocol-Version: 1`. Manager können Repository-Token mit einem verpflichtenden zukünftigen Ablaufzeitpunkt ausstellen und sie in der Repository-Verwaltung widerrufen.

```sh
curl --fail-with-body \
  'https://git-tracker.api.delino.io/gittracker.v1.GitTrackerService/Upload' \
  -H "Authorization: Bearer ${GIT_TRACKER_TOKEN}" \
  -H 'Content-Type: application/json' \
  -H 'Connect-Protocol-Version: 1' \
  --data-binary @upload.json
```

`upload.json` enthält `context`, `dataset` und `rows`:

```json
{
  "context": {
    "repositoryId": "019b0000-0000-7000-8000-000000000001",
    "executionId": "my-historical-run",
    "commit": "0123456789012345678901234567890123456789",
    "branch": "main"
  },
  "dataset": {
    "id": "size",
    "name": "Size",
    "columns": [{ "id": "bytes", "name": "Bytes", "unit": "B", "direction": "DIRECTION_LOWER", "absoluteThreshold": "1024" }]
  },
  "rows": [{ "id": "bundle", "name": "Bundle", "values": { "bytes": "1048576" } }]
}
```

Rufen Sie `/gittracker.v1.GitTrackerService/Complete` mit demselben `context` auf. Fügen Sie für PR-Uploads `pullRequest` hinzu und verwenden Sie den Basisbranch und den Head-Commit des PRs. `baselineCommit` wählt einen expliziten vollständigen Basis-SHA aus. Historische Commits verwenden dieselbe Upload-API. Wandeln Sie Dezimalzeichenfolgen vor dem Senden nicht über JavaScript-`Number` um.

Fügen Sie Text, der mit einem Upload oder Abschluss gesendet wird, kein NUL-Zeichen (`\u0000`) hinzu. Git Tracker weist es mit einem Clientfehler zurück, der das betroffene Feld benennt. Andere unterstützte Unicode-Texte und Steuerzeichen bleiben gültig.

## Regeln für Ausführung und Vergleich

Der erste Upload schreibt die Konfiguration jedes Datensatzes für diese Ausführung fest. Eine spätere abweichende Konfiguration wird abgelehnt. Das erneute Senden einer vorhandenen Zeile mit identischen numerischen Werten ist idempotent. Widersprüchliche Werte werden abgelehnt. Ein Batch ist atomar: Bei einer ungültigen oder widersprüchlichen Zeile wird nichts aus dieser Anfrage gespeichert.

Daten erscheinen sofort mit dem Status **Messung läuft**. Sie werden erst nach dem Abschluss repräsentativ. Das repräsentative Ergebnis für einen Commit ist seine zuletzt registrierte abgeschlossene Ausführung, einschließlich Ausführungen mit Regressionen. Eine ältere Ausführung, die spät fertig wird, ersetzt keine neuer registrierte abgeschlossene Ausführung. Ein wiederholter Abschluss ändert weder die Abschlusszeit noch die Reihenfolge.

Nach sechs Stunden ohne neu gespeicherte Messdaten erhält eine nicht abgeschlossene Ausführung den Status **Unterbrochen**. Identische erneute Übertragungen setzen diesen Timer nicht zurück. Abgeschlossene und unterbrochene Ausführungen lehnen weitere Uploads ab; unterbrochene Ausführungen lehnen auch den Abschluss ab. Starten Sie stattdessen eine neue Ausführung. Daten bleiben erhalten, bis Sie sie löschen.

Automatische PR-Vergleiche schreiben beim ersten Upload den Merge-Base-Commit und dessen repräsentative abgeschlossene Ausführung fest. Eine fehlende Vergleichsbasis bleibt nicht verfügbar, auch wenn sie später hochgeladen wird. Automatische Push-Vergleiche prüfen für jede Zeile separat bis zu 1.000 nächstgelegene First-Parent-Vorfahren und verwenden den nächstgelegenen gemessenen Vorfahren, dessen Ausführung bis zum Stichtag des ersten Uploads abgeschlossen war. Wenn innerhalb dieses Bereichs keine geeignete Zeilenmessung vorhanden ist, ist der Vergleich nicht verfügbar. Spätere Jobs verwenden denselben Stichtag. Die Tabelle weist jeden tatsächlich verwendeten Quell-Commit und jede Ausführung aus. Sie können einen expliziten Basis-SHA festlegen, um mit einem älteren Commit zu vergleichen; explizite Vergleiche greifen nicht auf Vorfahren einzelner Zeilen zurück.

Der Zwei-Commit-Vergleich der Website bevorzugt für jeden ausgewählten Commit die zuletzt registrierte abgeschlossene Branch-Ausführung. Wenn für einen Commit keine abgeschlossene Branch-Ausführung vorhanden ist, verwendet er die zuletzt registrierte abgeschlossene Pull-Request-Ausführung für diesen Commit. Die ausgewählte Zielausführung liefert ihre gespeicherte Datensatzkonfiguration. Neue Zeilen/Spalten sowie fehlende oder inkompatible Daten sind nicht vergleichbar und verursachen keinen regressionsbedingten Fehlschlag. Gelöschte Basisdaten werden nicht rekonstruiert. Vor dem Abschluss gilt eine nicht vorhandene Zeile als noch nicht erfasst; nach dem Abschluss gilt sie als fehlend.

Diagramme verwenden gleichmäßig verteilte Commits in First-Parent-Reihenfolge. Sie verbinden tatsächlich gemessene Punkte über übersprungene Commits hinweg und schreiben niemals Werte fort oder erfinden sie. Filtern Sie nach Datensatz, Branch, Spalte, Elementen, Datumsbereich und Commit-Bereich. Verwenden Sie die Datentabelle für exakte Werte und per Tastatur zugängliche Links zu Ausführungen.

## Löschen und Fehlerbehebung

Manager können nach Bestätigung die Messwerte eines Repositorys, einen Datensatz oder eine einzelne Ausführung dauerhaft löschen. Beim Löschen einer unbekannten Ausführung oder eines Datensatzes ohne aktuelle Messwerte wird gemeldet, dass das Ziel nicht gefunden wurde. Das Löschen eines Datensatzes betrifft nur Ausführungen, die ihn enthielten; andere Ausführungen können diesen Datensatz weiterhin hochladen. Ein Vergleich, dessen Basismesswerte gelöscht wurden, wird nicht verfügbar, während eine Vergleichsbasis, die den Datensatz nie enthielt, ihn als neue Daten behandelt. Wenn Delcore die Löschung eines Spaces abschließt, löscht Git Tracker die Verbindungen, Token, Konfiguration und den Verlauf des Spaces.

Überprüfen Sie bei einem Berechtigungsfehler den ausgewählten Space und Ihre aktuelle Git-Tracker-Rolle. Prüfen Sie bei Upload-Konflikten die Ausführungsidentität, die Datensatzkonfiguration und die vollständigen Zeilenwerte. Verwenden Sie für unterbrochene oder abgeschlossene Ausführungen eine neue Ausführungs-ID. Versuchen Sie es bei Netzwerk-/Serverfehlern erneut und bewahren Sie die Korrelations-ID der Antwort auf, wenn Sie um Hilfe bitten. Teilen Sie niemals Token, Anmeldedaten oder private Messwerte in öffentlichen Berichten.

Messwerte aus PR-Ausführungen bleiben einsehbar, liefern aber weder Punkte für Upstream-Branch-Diagramme noch automatische Vergleichsbasen. Dadurch wird verhindert, dass Messwerte aus Forks die Daten oder die Konfiguration des Basisbranches für einen gemeinsamen Commit-SHA ersetzen.
