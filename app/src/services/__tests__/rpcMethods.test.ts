import * as fs from 'node:fs';
import * as path from 'node:path';
import { describe, expect, test } from 'vitest';

import { CORE_RPC_METHODS, LEGACY_METHOD_ALIASES, normalizeRpcMethod } from '../rpcMethods';

describe('rpcMethods catalog', () => {
  describe('normalizeRpcMethod', () => {
    test('resolves all legacy aliases to their canonical core method', () => {
      for (const [legacyMethod, coreMethod] of Object.entries(LEGACY_METHOD_ALIASES)) {
        expect(normalizeRpcMethod(legacyMethod)).toBe(coreMethod);
      }
    });

    test('transforms auth methods by replacing dots with underscores', () => {
      expect(normalizeRpcMethod('neppy.auth.login')).toBe('neppy.auth_login');
      expect(normalizeRpcMethod('neppy.auth.get.state')).toBe('neppy.auth_get_state');
      expect(normalizeRpcMethod('neppy.auth.a.b.c')).toBe('neppy.auth_a_b_c');
    });

    test('returns unmapped or unrecognized methods unchanged', () => {
      expect(normalizeRpcMethod('neppy.threads_list')).toBe('neppy.threads_list');
      expect(normalizeRpcMethod('neppy.unknown_method')).toBe('neppy.unknown_method');
      expect(normalizeRpcMethod('')).toBe('');
      expect(normalizeRpcMethod('random_string')).toBe('random_string');
    });

    test('trims whitespace and converts to lower case', () => {
      expect(normalizeRpcMethod('  OpenHuman.Auth.Login  ')).toBe('neppy.auth_login');
      expect(normalizeRpcMethod('  OPENHUMAN.GET_CONFIG ')).toBe(CORE_RPC_METHODS.configGet);
      expect(normalizeRpcMethod('OpenHuman.Unrecognized_Status  ')).toBe(
        'neppy.unrecognized_status'
      );
      expect(normalizeRpcMethod('   some_RANDOM_method  ')).toBe('some_random_method');
    });
  });

  test('legacy aliases point at canonical method values', () => {
    expect(LEGACY_METHOD_ALIASES['neppy.update_model_settings']).toBe(
      CORE_RPC_METHODS.inferenceUpdateModelSettings
    );
    expect(LEGACY_METHOD_ALIASES['neppy.workspace_onboarding_flag_set']).toBe(
      CORE_RPC_METHODS.configWorkspaceOnboardingFlagSet
    );
  });

  describe('MCP client legacy alias resolution (Sentry CORE-RUST-DW/DV/DT/DS/DR)', () => {
    test('mcp_clients.list resolves to mcp_clients_installed_list', () => {
      expect(normalizeRpcMethod('mcp_clients.list')).toBe(CORE_RPC_METHODS.mcpClientsInstalledList);
    });

    test('neppy.mcp_clients_list resolves to mcp_clients_installed_list', () => {
      expect(normalizeRpcMethod('neppy.mcp_clients_list')).toBe(
        CORE_RPC_METHODS.mcpClientsInstalledList
      );
    });

    test('neppy.mcp_list resolves to mcp_clients_installed_list', () => {
      expect(normalizeRpcMethod('neppy.mcp_list')).toBe(CORE_RPC_METHODS.mcpClientsInstalledList);
    });

    test('neppy.mcp_servers_list resolves to mcp_clients_installed_list', () => {
      expect(normalizeRpcMethod('neppy.mcp_servers_list')).toBe(
        CORE_RPC_METHODS.mcpClientsInstalledList
      );
    });

    test('neppy.tool_registry_call resolves to mcp_clients_tool_call', () => {
      expect(normalizeRpcMethod('neppy.tool_registry_call')).toBe(
        CORE_RPC_METHODS.mcpClientsToolCall
      );
    });

    test('dotted tool_registry.diagnostics resolves to the canonical method (#3294)', () => {
      expect(normalizeRpcMethod('tool_registry.diagnostics')).toBe(
        CORE_RPC_METHODS.toolRegistryDiagnostics
      );
      expect(CORE_RPC_METHODS.toolRegistryDiagnostics).toBe('neppy.tool_registry_diagnostics');
    });

    test('canonical mcp_clients_installed_list passes through unchanged', () => {
      expect(normalizeRpcMethod('neppy.mcp_clients_installed_list')).toBe(
        'neppy.mcp_clients_installed_list'
      );
    });

    test('canonical mcp_clients_tool_call passes through unchanged', () => {
      expect(normalizeRpcMethod('neppy.mcp_clients_tool_call')).toBe('neppy.mcp_clients_tool_call');
    });
  });

  describe('health legacy alias resolution (Sentry CORE-RUST-FG / CORE-RUST-G0)', () => {
    test('health_snapshot resolves to neppy.health_snapshot', () => {
      expect(normalizeRpcMethod('health_snapshot')).toBe(CORE_RPC_METHODS.healthSnapshot);
    });

    test('neppy.system_info resolves to neppy.health_system_info (Sentry CORE-RUST-G0)', () => {
      // Older clients called neppy.system_info before the method was
      // namespaced under health as neppy.health_system_info.
      expect(normalizeRpcMethod('neppy.system_info')).toBe(CORE_RPC_METHODS.healthSystemInfo);
    });

    test('canonical health_system_info passes through unchanged', () => {
      expect(normalizeRpcMethod('neppy.health_system_info')).toBe('neppy.health_system_info');
    });
  });

  describe('openhuman. -> neppy. prefix rebrand', () => {
    test('legacy openhuman. prefix is rewritten to neppy.', () => {
      expect(normalizeRpcMethod('openhuman.memory_doc_put')).toBe('neppy.memory_doc_put');
      expect(normalizeRpcMethod('neppy.memory_doc_put')).toBe('neppy.memory_doc_put');
    });

    test('legacy-prefixed aliases still resolve through the alias table', () => {
      expect(normalizeRpcMethod('openhuman.get_config')).toBe(CORE_RPC_METHODS.configGet);
      expect(normalizeRpcMethod('openhuman.ping')).toBe(CORE_RPC_METHODS.corePing);
      expect(normalizeRpcMethod('openhuman.channels.list')).toBe(CORE_RPC_METHODS.channelsList);
    });

    test('dotted auth spelling resolves under either prefix', () => {
      expect(normalizeRpcMethod('openhuman.auth.oauth_connect')).toBe('neppy.auth_oauth_connect');
      expect(normalizeRpcMethod('neppy.auth.oauth_connect')).toBe('neppy.auth_oauth_connect');
    });
  });

  describe('channels legacy alias resolution (Sentry OPENHUMAN-CORE-1Y / OPENHUMAN-CORE-1Z)', () => {
    test('dotted channel list aliases resolve to channels_list', () => {
      expect(normalizeRpcMethod('channels.list')).toBe(CORE_RPC_METHODS.channelsList);
      expect(normalizeRpcMethod('neppy.channels.list')).toBe(CORE_RPC_METHODS.channelsList);
    });

    test('canonical channels_list passes through unchanged', () => {
      expect(normalizeRpcMethod('neppy.channels_list')).toBe('neppy.channels_list');
    });
  });

  test('catalog canonical methods exist in core schema registry (drift guard)', () => {
    const schemaSources = [
      fs.readFileSync(
        path.resolve(__dirname, '../../../../src/neppy/config/schemas/schema_defs.rs'),
        'utf8'
      ),
      fs.readFileSync(
        path.resolve(__dirname, '../../../../src/neppy/inference/provider/schemas.rs'),
        'utf8'
      ),
      fs.readFileSync(
        path.resolve(__dirname, '../../../../src/neppy/inference/schemas.rs'),
        'utf8'
      ),
      fs.readFileSync(
        path.resolve(__dirname, '../../../../src/neppy/inference/local/schemas.rs'),
        'utf8'
      ),
      fs.readFileSync(
        path.resolve(__dirname, '../../../../src/neppy/inference/embeddings/schemas.rs'),
        'utf8'
      ),
      fs.readFileSync(
        path.resolve(__dirname, '../../../../src/neppy/mcp/registry/schemas.rs'),
        'utf8'
      ),
      fs.readFileSync(
        path.resolve(__dirname, '../../../../src/neppy/tools/registry/schemas.rs'),
        'utf8'
      ),
      fs.readFileSync(
        path.resolve(__dirname, '../../../../src/neppy/platform/health/schemas.rs'),
        'utf8'
      ),
      fs.readFileSync(
        path.resolve(__dirname, '../../../../src/neppy/channels/controllers/schemas.rs'),
        'utf8'
      ),
      // The channels_* namespace/function literals now live in the vendored
      // tinychannels crate (`ChannelControllerSchema`), not in the thin
      // `src/neppy/channels/controllers/schemas.rs` adapter above, which
      // only converts from it (#4557 "Use tinychannels provider
      // implementations") — read both so this drift guard still sees them.
      fs.readFileSync(
        path.resolve(__dirname, '../../../../vendor/tinychannels/src/controllers/schemas.rs'),
        'utf8'
      ),
    ].join('\n');

    for (const method of Object.values(CORE_RPC_METHODS)) {
      // core.* methods (e.g. core.ping) are special dispatch methods, not in the schema catalog.
      if (!method.startsWith('neppy.')) continue;
      const methodRoot = method.slice('neppy.'.length);
      const namespace = methodRoot.startsWith('inference_')
        ? 'inference'
        : methodRoot.startsWith('embeddings_')
          ? 'embeddings'
          : methodRoot.startsWith('providers_')
            ? 'providers'
            : methodRoot.startsWith('mcp_clients_')
              ? 'mcp_clients'
              : methodRoot.startsWith('health_')
                ? 'health'
                : methodRoot.startsWith('channels_')
                  ? 'channels'
                  : methodRoot.startsWith('tool_registry_')
                    ? 'tool_registry'
                    : 'config';
      const fnName = methodRoot.slice(`${namespace}_`.length);
      expect(schemaSources).toContain(`namespace: "${namespace}"`);
      expect(schemaSources).toContain(`function: "${fnName}"`);
    }
  });
});
