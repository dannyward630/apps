# Git Tracker

Acompanhe medições numéricas de commits Git em [git-tracker.delino.io](https://git-tracker.delino.io). O Git Tracker é gratuito. Ele oferece suporte a repositórios públicos e privados do GitHub, incluindo pull requests de forks. Use-o para tamanhos de arquivos, tempos, taxa de transferência, contagens e outras medições escalares.

## Conecte um repositório

1. Entre com sua conta Delino e selecione seu espaço pessoal ou de organização.
2. Peça a um proprietário do espaço para conceder a permissão View, Edit ou Manage do Git Tracker por meio dos grupos do espaço. View permite leituras. Edit adiciona uploads e conclusão. Manage adiciona conexão, gerenciamento de tokens e exclusão permanente.
3. Instale o Git Tracker GitHub App para o repositório.
4. Abra **Gerenciar repositórios**. Insira `owner/repository` e um token de acesso do GitHub para sua conta GitHub vinculada com acesso de administrador ao repositório. O serviço verifica sua autoridade no GitHub e não armazena esse token de acesso.
5. Conecte o repositório. Você também precisa de autoridade de Owner/Admin no espaço Delino. Um repositório só pode estar conectado a um espaço por vez. Copie o ID de conexão no gerenciamento de repositórios para chamadas da Action e da API. Após a desconexão, outro espaço autorizado pode conectá-lo com um novo ID de conexão; o histórico retido do espaço antigo permanece privado.

As medições de repositórios públicos podem ser lidas sem entrar na conta. Medições privadas exigem associação ao espaço e permissão de leitura do Git Tracker. O acesso ao espaço se aplica a todos os repositórios conectados nesse espaço.

Desconectar ou remover o App interrompe os uploads e torna privado o histórico retido. Membros autorizados ainda podem lê-lo. Uma alteração confirmada de público para privado torna os dados históricos privados. Se o GitHub não conseguir confirmar a visibilidade atual, o Git Tracker mantém a última visibilidade confirmada até que a sincronização seja bem-sucedida.

## Defina seu conjunto de dados

Mantenha uma configuração YAML no seu repositório. Os IDs são estáveis; os nomes podem mudar sem perda de histórico. Todas as colunas de uma linha devem ser enviadas juntas.

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

Envie linhas JSON, por exemplo, uma tabela de tamanhos de arquivos no estilo SWC:

```json
{
  "rows": [
    { "id": "jquery", "name": "jQuery", "values": { "original": 285314, "minified": 87533, "gzip": 30321 } },
    { "id": "three", "name": "Three.js", "values": { "original": 1321000, "minified": 675000, "gzip": 170000 } }
  ]
}
```

A Action preserva a representação decimal do JSON sem conversão para ponto flutuante binário. Clientes que usam o Connect diretamente enviam valores decimais como strings, como `"9007199254740993.0000000000000001"`. Números negativos e decimais finitos são aceitos. O arredondamento na exibição nunca altera as decisões de regressão.

As unidades aceitas são `B`, `kB`, `MB`, `GB`, `KiB`, `MiB`, `GiB`; `ns`, `μs`, `ms`, `s`; `count`, `%`, `unitless`; e `ops/s`, `B/s`, `kB/s`, `MB/s`, `GB/s`, `KiB/s`, `MiB/s`, `GiB/s`. Unidades compatíveis são convertidas antes da comparação. Tipos incompatíveis não são comparáveis. Por exemplo, 1048576 B é exibido como 1 MiB; os valores exatos permanecem visíveis nas tabelas.

As direções são `lower`, `higher` e `neutral`. Colunas neutras nunca produzem regressões. Os limites se aplicam a todas as linhas da coluna. Com limites absoluto e percentual, exceder qualquer um deles é uma regressão; a igualdade não é. Sem limite, as alterações são exibidas, mas não causam falhas por regressão. Somente alterações na direção de piora são avaliadas.

A variação percentual é `(new - baseline) / abs(baseline) * 100`. Uma referência igual a zero não tem variação percentual. Um limite absoluto ainda pode determinar o resultado; uma regra apenas percentual não emite uma avaliação. As comparações usam a configuração armazenada da execução de destino, não uma configuração posterior.

## Use a GitHub Action oficial

Fixe `delinoio/apps/actions/git-tracker` em um SHA de commit completo revisado. Substitua os placeholders abaixo pelo ID de conexão do Git Tracker, pela revisão de checkout e pela revisão da Action. Conceda `id-token: write` para OIDC. Um token de upload do repositório é uma alternativa; armazene-o como um segredo do GitHub e passe `token`.

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

Jobs independentes compartilham o ID de execução padrão, `repository_id/run_id/run_attempt`, e podem enviar conjuntos de linhas distintos. Use um novo ID de execução para reexecuções. O OIDC do Actions exige a identidade padrão; IDs de execução personalizados são destinados a clientes que usam tokens de repositório. Para um job de pull request cuja credencial OIDC pertence ao repositório conectado, defina a entrada `baseline` da Action para usar um SHA de commit completo explícito. Uploads OIDC de forks são limitados ao head do PR e não podem fornecer substituições de referência nem gravar no histórico de push do upstream.

Cada job de upload avalia apenas as próprias linhas enviadas. Por padrão, as regressões são relatadas sem causar falha no CI. Defina `fail-on-regression: 'true'` para fazer esse job falhar em caso de regressões. Comparações ausentes não causam falha no CI. O job de conclusão apenas indica que a medição terminou; ele não envia o estado de sucesso/falha/cancelamento do CI nem executa uma verificação agregada de aprovação. Uma conclusão vazia é válida quando todos os jobs foram ignorados.

Erros de rede e falhas transitórias da API são repetidos no máximo três vezes. Por padrão, o job falha após as novas tentativas. Defina `warn-on-api-error: 'true'` para emitir um aviso e passar em vez disso. Falhas em comentários de PR apenas emitem avisos; elas não invalidam os dados salvos nem causam, por si só, falha no CI. A Action registra um link de detalhes e escreve um resumo do job.

Um comentário no PR reflete os resultados cumulativos da execução registrada mais recente no commit mais recente do PR. Execuções mais antigas permanecem disponíveis no histórico da web.

## Chame a API Connect

A origem da API é `https://git-tracker.api.delino.io`. As solicitações JSON do Connect usam `Content-Type: application/json` e `Connect-Protocol-Version: 1`. Gerentes podem emitir tokens de repositório com uma expiração futura obrigatória e revogá-los no gerenciamento de repositórios.

```sh
curl --fail-with-body \
  'https://git-tracker.api.delino.io/gittracker.v1.GitTrackerService/Upload' \
  -H "Authorization: Bearer ${GIT_TRACKER_TOKEN}" \
  -H 'Content-Type: application/json' \
  -H 'Connect-Protocol-Version: 1' \
  --data-binary @upload.json
```

`upload.json` contém `context`, `dataset` e `rows`:

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

Chame `/gittracker.v1.GitTrackerService/Complete` com o mesmo `context`. Para uploads de PR, inclua `pullRequest` e use a branch base e o commit head do PR. `baselineCommit` seleciona um SHA completo explícito de referência. Commits históricos usam a mesma API de upload. Não converta strings decimais por meio de `Number` do JavaScript antes de enviá-las.

Não inclua um caractere NUL (`\u0000`) no texto enviado em um upload ou uma conclusão. O Git Tracker o rejeita com um erro de cliente que identifica o campo afetado. Outros textos Unicode e caracteres de controle aceitos permanecem válidos.

## Regras de execução e comparação

O primeiro upload fixa a configuração de cada conjunto de dados para essa execução. Uma configuração posterior diferente é rejeitada. Repetir uma linha existente com valores numéricos idênticos é idempotente. Valores conflitantes são rejeitados. Um lote é atômico: uma linha inválida ou conflitante impede que qualquer dado daquela solicitação seja salvo.

Os dados aparecem imediatamente como **Medindo**. Eles só se tornam representativos após a conclusão. O resultado representativo de um commit é sua execução concluída registrada mais recentemente, incluindo execuções com regressões. Uma execução mais antiga que termina tarde não substitui uma execução concluída registrada mais recentemente. Repetir a conclusão não altera o horário de conclusão nem a ordenação.

Após seis horas sem novos dados de medição salvos, uma execução inacabada passa a **Interrompida**. Retransmissões idênticas não reiniciam esse temporizador. Execuções concluídas e interrompidas rejeitam uploads adicionais; execuções interrompidas também rejeitam a conclusão. Inicie uma nova execução em vez disso. Os dados permanecem até que você os exclua.

Comparações automáticas de PR fixam o commit de merge-base e sua execução concluída representativa no primeiro upload. Uma referência ausente permanece indisponível mesmo se for enviada posteriormente. Comparações automáticas de push verificam até os 1.000 ancestrais mais próximos pela cadeia do primeiro pai separadamente para cada linha e usam o ancestral medido mais próximo concluído até o momento de corte do primeiro upload. Se não houver nenhuma medição de linha elegível nesse intervalo, a comparação ficará indisponível. Jobs posteriores usam esse mesmo momento de corte. A tabela identifica cada commit e execução de origem efetivamente usados. Você pode definir um SHA de referência explícito para comparar com um commit mais antigo; comparações explícitas não usam a busca alternativa de ancestrais por linha.

A comparação de dois commits do site dá preferência à execução de branch concluída registrada mais recentemente para cada commit selecionado. Se um commit não tiver uma execução de branch concluída, ela usa a execução de pull request concluída registrada mais recentemente para esse commit. A execução de destino selecionada fornece sua configuração de conjunto de dados armazenada. Novas linhas/colunas e dados ausentes ou incompatíveis não são comparáveis e não causam falha por regressão. Dados de referência excluídos não são reconstruídos. Antes da conclusão, uma linha ausente é considerada não coletada; após a conclusão, é considerada faltante.

Os gráficos usam commits igualmente espaçados na ordem do primeiro pai. Eles conectam pontos medidos reais através de commits ignorados e nunca propagam nem inventam valores. Filtre por conjunto de dados, branch, coluna, itens, datas e intervalo de commits. Use a tabela de dados para valores exatos e links de execução acessíveis por teclado.

## Exclusão e solução de problemas

Gerentes podem excluir permanentemente as medições de um repositório, um conjunto de dados ou uma execução após confirmação. Excluir uma execução desconhecida ou um conjunto de dados sem medições atuais informa que o destino não foi encontrado. Excluir um conjunto de dados afeta apenas as execuções que o continham; outras execuções ainda podem enviar esse conjunto de dados. Uma comparação cujas medições de referência foram excluídas fica indisponível, enquanto uma referência que nunca continha o conjunto de dados o trata como dados novos. Quando o Delcore finaliza a exclusão do espaço, o Git Tracker exclui as conexões, os tokens, a configuração e o histórico do espaço.

Para um erro de permissão, verifique o espaço selecionado e sua função atual no Git Tracker. Para conflitos de upload, confirme a identidade da execução, a configuração do conjunto de dados e os valores completos das linhas. Para execuções interrompidas ou concluídas, use um novo ID de execução. Para erros de rede/servidor, tente novamente e guarde o ID de correlação da resposta ao pedir ajuda. Nunca compartilhe tokens, credenciais ou medições privadas em relatos públicos.

As medições de execuções de PR continuam disponíveis para inspeção, mas não fornecem pontos para o gráfico da branch upstream nem referências automáticas. Isso impede que medições de forks substituam os dados ou a configuração da branch de referência para um SHA de commit compartilhado.
