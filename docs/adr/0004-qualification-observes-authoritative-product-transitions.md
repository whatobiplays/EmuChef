# ADR 0004: Qualification observes authoritative product transitions

**Status:** Accepted

Physical qualification remains a development-only observer of the ordinary EmuChef workflow rather than a parallel workflow. Passive device interpretation lives behind a typed `device_observation` module, while one `qualification_session` module owns evidence-session ordering, persistence, recovery, invalidation, and candidate materialization. Trusted Tauri orchestration passes exact committed device, root, review, and real-execution results synchronously to the active session after the product operation commits; React only presents state and explicit operator actions.

This deliberately rejects frontend lifecycle reconstruction, a generic event bus, asynchronous qualification queues/journals, multiple active sessions, and re-querying product state to reconstruct history. Qualification-only failure never changes an already-successful product result, but any missed or unpersisted authoritative transition permanently fails closed for evidence.
