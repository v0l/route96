# Dynamic Labeling Config - Implementation Summary

## Issue #70 - Dynamic labeling config

### What was implemented

1. **Database Schema** (`migrations/20260313000000_label_models.sql`)
   - Created `label_models` table with columns: name (PK), model_type, config

2. **Database Methods** (`src/db.rs`)
   - Added `DbLabelModel` struct for database operations
   - Implemented `get_label_models()` to fetch all models from DB
   - Implemented `add_label_model()` to add/update models
   - Implemented `remove_label_model()` to delete models

3. **Admin API Endpoints** (`src/routes/admin.rs`)
   - `GET /admin/label-models` - List all configured label models
   - `POST /admin/label-models` - Add or update a label model
   - `DELETE /admin/label-models/{name}` - Remove a label model

4. **Configuration Integration** (`src/config_watcher.rs`)
   - Updated `build_settings()` to merge database models into settings
   - Database models are appended to config file models

### Supported Model Types

- **ViT (Vision Transformer)** - HuggingFace models
  - Example: `google/vit-base-patch16-224`, `Falconsai/nsfw_image_detection`
- **Generic LLM** - OpenAI-compatible API endpoints
  - Supports custom prompts and API keys

### Features

- Hot-reload support (changes take effect without restart)
- Optional fields: `label_exclude`, `min_confidence`
- Validation for required fields based on model type

### API Usage Examples

#### Add a ViT model
```bash
curl -X POST http://localhost:3000/admin/label-models \
  -H 'Authorization: Bearer <token>' \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "vit224",
    "model_type": "vit",
    "hf_repo": "google/vit-base-patch16-224",
    "min_confidence": 0.3
  }'
```

#### List all models
```bash
curl http://localhost:3000/admin/label-models \
  -H 'Authorization: Bearer <token>'
```

#### Remove a model
```bash
curl -X DELETE http://localhost:3000/admin/label-models/vit224 \
  -H 'Authorization: Bearer <token>'
```

### Next Steps

The implementation is complete and ready for review. A PR should be created from the branch `openhands/issue-70-dynamic-labeling-config`.
